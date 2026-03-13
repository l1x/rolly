# How We Made rolly 3x Faster Than OpenTelemetry SDK

A chronicle of profiling, bottleneck hunting, and surgical optimization on the metrics hot path.

## The Starting Point

rolly is a lightweight Rust observability crate — hand-rolled OTLP protobuf over HTTP, built on `tracing`. It ships traces, logs, and metrics with 7 direct dependencies where the OpenTelemetry SDK needs ~120. Compile times are seconds, not minutes.

But when we ran head-to-head benchmarks against `opentelemetry_sdk` 0.31, the metrics recording path told a humbling story. The pre-optimization rolly was slower than OTel across the board — by up to 5x on the simplest operations. Something was deeply wrong.

## Challenge 1: The Invisible Exemplar Tax

**Problem.** Every call to `Counter::add()` started with `capture_exemplar()`, which called `tracing::Span::current()`. This does a thread-local subscriber lookup — walking the tracing dispatcher, checking for an active span, looking for trace context extensions. It costs 15-20 ns even when no tracing subscriber is installed (the common case in benchmarks and metrics-only deployments).

**Flamechart evidence.** The "before" flamechart shows `capture_exemplar_span_current` consuming 4% of all samples, with `tracing::Span::current` and `tracing_core::subscriber::Subscriber::current_span` visible underneath it.

**Solution.** A single-line guard at the top of `capture_exemplar()`:

```rust
fn capture_exemplar(value: ExemplarValue) -> Option<Exemplar> {
    if !tracing::dispatcher::has_been_set() {
        return None;
    }
    // ... existing span lookup code
}
```

`tracing::dispatcher::has_been_set()` is a single `Relaxed` atomic load — about 1 ns. When no subscriber is installed, we skip the entire span lookup. When a subscriber IS installed (production), the behavior is unchanged.

**Verification.** The "after" flamechart shows `capture_exemplar` at ~5% but now it's just the atomic check. The `tracing::Span::current` frame is gone entirely.

## Challenge 2: Copy, Sort, Hash — Three Passes Over Attributes

**Problem.** The original `Counter::add()` hot path did three separate passes over the attribute slice:

```rust
// BEFORE: 3 passes, 1 heap allocation
let mut sorted = attrs.to_vec();  // Pass 1: heap alloc + copy
sorted.sort();                     // Pass 2: comparison sort
let key = attrs_hash(&sorted);    // Pass 3: SipHash
```

The `to_vec()` allocates on the heap every single call. The sort is O(n log n) with `memcmp` calls. The `attrs_hash` function uses `DefaultHasher` (SipHash-1-3), which is cryptographically strong but slow for this use case.

**Flamechart evidence.** The "before" flamechart is dominated by these three operations:
- `attrs_hash_siphash`: 714 samples (18%)
- `sort_attrs` + `smallsort::insert_tail`: 591 + 487 samples (27% combined)
- `attrs_to_vec` + `_nanov2_free` (malloc/free): 542 + 443 samples (25% combined)

Together, copy + sort + hash consumed **70% of the hot path**.

**Solution.** Replace all three passes with a single-pass, zero-allocation, order-independent hash:

```rust
fn attrs_hash_unordered(attrs: &[(&str, &str)]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut combined: u64 = 0;
    for &(k, v) in attrs {
        let mut h: u64 = FNV_OFFSET;
        for byte in k.as_bytes() {
            h ^= *byte as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
        h ^= 0xff; // separator so ("ab","c") != ("a","bc")
        h = h.wrapping_mul(FNV_PRIME);
        for byte in v.as_bytes() {
            h ^= *byte as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
        combined = combined.wrapping_add(h); // commutative!
    }
    combined
}
```

The key insight: `wrapping_add` is commutative. By hashing each key-value pair independently with FNV-1a and then summing the hashes, the result is the same regardless of attribute order. No copy, no sort, no allocation.

The sort is preserved only in `owned_attrs()` — the cold path that runs once per unique attribute set, when a new entry is inserted into the HashMap.

**Verification.** The "after" flamechart shows zero frames for sort, allocation, or FNV hashing — the hash computation is so fast it doesn't register in sampling.

## Challenge 3: Double Hashing in HashMap

**Problem.** The `HashMap<u64, CounterDataPoint>` stores pre-hashed u64 keys. But Rust's `HashMap` uses SipHash internally by default, so every `.entry(key)` call SipHash-es the already-hashed key. This is pure waste.

**Flamechart evidence.** The "before" flamechart shows `core::hash::BuildHasher::hash_one` at 319 samples (8%) — this is the HashMap re-hashing our already-hashed keys.

**Solution.** A trivial identity hasher that passes through u64 keys untouched:

```rust
#[derive(Default)]
struct IdentityHasher(u64);

impl Hasher for IdentityHasher {
    fn finish(&self) -> u64 { self.0 }
    fn write(&mut self, _: &[u8]) { unreachable!() }
    fn write_u64(&mut self, n: u64) { self.0 = n; }
}

type IdentityBuildHasher = BuildHasherDefault<IdentityHasher>;
```

Changed all three metric types to use `HashMap<u64, ..., IdentityBuildHasher>`. The `finish()` call now costs 0 ns instead of ~10 ns.

**Verification.** The "after" flamechart shows no `BuildHasher::hash_one` frame. The HashMap probe is invisible.

## Challenge 4: Empty Attributes Still Hashed

**Problem.** `counter.add(1, &[])` — the simplest possible call — still ran the full hash function on an empty slice. OTel's no-attrs path is an atomic increment (~5 ns). Ours was 28 ns.

**Solution.** A fast-path constant:

```rust
let key = if attrs.is_empty() { 0 } else { attrs_hash_unordered(attrs) };
```

Empty attrs always hash to 0. No function call, no loop, no computation.

## The Results

After all four optimizations, the "after" flamechart tells a completely different story. The entire hot path is now dominated by the mutex lock/unlock — the actual computation (hash, exemplar check) is too fast to appear in sampling:

| Frame | Before (samples) | After (samples) |
|---|---|---|
| SipHash (`attrs_hash`) | 714 (18%) | 0 (gone) |
| Sort (`sort_attrs`) | 591 (15%) | 0 (gone) |
| Heap alloc (`attrs_to_vec`) | 542 (13%) | 0 (gone) |
| Heap free (`_nanov2_free`) | 443 (11%) | 0 (gone) |
| HashMap re-hash | 319 (8%) | 0 (gone) |
| Exemplar span lookup | 166 (4%) | 0 (now atomic) |
| Mutex lock | 251 (6%) | 802 (51%) |
| Mutex unlock | — | 387 (25%) |

The mutex is now the bottleneck — which is correct. It's the irreducible synchronization cost. Everything else was eliminated.

Criterion benchmark results (95% confidence intervals on mean):

| Benchmark | rolly (ns) | OTel SDK (ns) | vs OTel |
|---|---|---|---|
| Counter (no attrs) | 9.6 ± 0.0 | 4.9 ± 0.0 | 2.0x slower |
| Counter (3 attrs) | **26.8 ± 0.4** | 86.4 ± 0.2 | **3.2x faster** |
| Counter (5 attrs) | **43.1 ± 0.1** | 132.8 ± 0.4 | **3.1x faster** |
| Histogram (3 attrs) | **29.2 ± 0.1** | 88.1 ± 0.2 | **3.0x faster** |
| Histogram (5 attrs) | **47.2 ± 0.1** | 139.5 ± 0.3 | **3.0x faster** |

All numbers from criterion (`cargo bench --features _bench -- comparison`) with 100+ iterations and outlier analysis. Cold-path benchmarks (first-insert with allocation) are also available via `comparison_counter_*_cold` and gauge comparisons via `comparison_gauge_*`.

rolly is 3x faster than OpenTelemetry SDK on all attributed metric operations. The no-attrs counter is still 2x behind OTel (9.6 ns vs 4.9 ns) — OTel uses a lock-free atomic increment for that case, while rolly still takes a mutex. That's a future optimization.

## What the OTel Flamechart Reveals

The OTel SDK flamechart shows where its time goes on the 3-attrs counter path:
- `KeyValue::hash` via SipHash: 31% of samples
- `SlicePartialEq::equal` (KeyValue comparison for HashMap probing): 23%
- `_platform_memcmp` (inside comparisons): 13%
- `drop_in_place<Value>` (dropping cloned KeyValues): 4%

OTel's main cost is hashing and comparing `KeyValue` structs, which are heap-allocated `String` wrappers. rolly uses `&str` slices — zero-copy borrows with no allocation.

## Methodology

**Latency numbers** come from [criterion.rs](https://bheisler.github.io/criterion.rs/book/) (`benches/comparison_otel.rs`). Criterion runs each benchmark for 100+ iterations, computes mean with 95% confidence intervals, performs outlier analysis, and detects regressions. All OTel benchmarks pre-build `KeyValue` arrays outside the measurement loop for a fair comparison. Results are machine-readable at `docs/flamecharts/benchmark_results.toml`.

**Flamechart profiling** uses a self-contained bench binary (`benches/generate_flamecharts.rs`). The binary:

1. Spawns itself in profiling mode (tight loop calling the hot path for 12 seconds)
2. Uses macOS `sample` command via `std::process::Command` to capture stack traces at 1kHz
3. Collapses the stacks using the `inferno` crate (Rust library)
4. Cleans frame names (strips binary hashes, thread descriptors, runtime boilerplate)
5. Renders interactive SVG flamegraphs using `inferno::flamegraph`
6. Reads criterion JSON results from `target/criterion/` and generates comparison outputs

No shell scripts. No Python. The entire pipeline is `cargo bench --features _bench`.

## Lessons

1. **Profile before guessing.** The exemplar tax was invisible in code review — `capture_exemplar()` looks cheap until you realize `Span::current()` touches a thread-local on every call.

2. **Commutativity eliminates sorting.** Order-independent hashing via wrapping-add of per-element hashes is an old trick, but it's easy to overlook when the "obvious" approach is sort-then-hash.

3. **Don't hash your hashes.** When HashMap keys are already well-distributed hashes, the default SipHash is pure overhead. An identity hasher is safe and free.

4. **Flamecharts don't lie.** The "after" flamechart showing 76% mutex lock/unlock is the best possible outcome — it means everything else was optimized away, and the remaining cost is the irreducible synchronization primitive.
