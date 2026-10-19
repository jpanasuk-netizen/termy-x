# Linux io_uring (tokio-uring / glommio) — verdict: DO NOT ADOPT

Agent linux-l3, 2026-10-19. Base `bc20ab0`. Scope: the X network-fetch /
provider pipeline only. Verdict: **no**. io_uring has no hot path to sit on
in Termy X today; adding tokio-uring or glommio would be decoration that
imports a second executor model, and in tokio-uring's case an unmaintained
one. A justified "no" with evidence, per the lane brief. No code changed,
no dependency added, Windows untouched, Linux build untouched.

## 1. Where the time actually goes (hot-path analysis)

Every X read/write in `crates/x_core` and the X panel reaches the network
in exactly one of these ways:

| Path | Code | Mechanism | Wall-clock scale |
|---|---|---|---|
| Timeline / search / profile / trends / post / like / RT | `x_core::io::SystemRunner` (`io.rs:422,537`) → `providers.rs` `opencli_read`/`engage_opencli` | **Subprocess**: spawn `opencli` (Node CLI) → Browser Bridge → a real Chrome instance driving x.com, JSON back over the child's stdout pipe | **5–25 s per call** (measured on the PC: post probe 5.7–8.9 s after the OpenCLI speed patch, `docs/OPENCLI-SPEED.md`; engage 8–11 s; post timeout budget had to be raised to 150 s to cover OpenCLI's own 90 s success-toast poll, commit `693edcd`) |
| AI draft variants (Compose) | `x_core::ai::draft_variants` → `UreqHttp` (`io.rs:715`) | **Blocking** `ureq` HTTPS/HTTP to an OpenAI-compatible endpoint (default: local Ollama `127.0.0.1:11434`) on a GPUI background thread | Server/LLM-bound: hundreds of ms to seconds; request body is a few hundred bytes |
| Official-API fallback reads | `providers.rs:1971,2149` via `Http` trait | Same blocking `ureq` agent | Remote-API latency (10s–100s of ms) + TLS |
| Tweet/avatar image fetch | `x_panel/mod.rs:3870 fetch_image_rgba`, spawned per-image on `cx.background_executor()` (`mod.rs:5209`) | Blocking `ureq::get`, 5 MB cap, then `image::load_from_memory` decode | Network + JPEG/PNG/WebP **decode** dominate; a handful of concurrent images, user-scroll-paced |
| Small file I/O | `drafts.rs`, `premium.rs:233`, `connect.rs:226`, `providers.rs:1085` (append-only debug/perf log) | `std::fs` read/write of KB-sized config/cache files, a few times per session | ~1 µs per 4 KiB read (measured below) |
| Workspace persistence | `desktop_app::workspace_store` | sqlx/SQLite on its **own** dedicated current-thread tokio runtime (`workspace_store.rs:93`) | SQLite does its own file I/O inside libsqlite3; not X pipeline, and sqlx-sqlite cannot be re-pointed at io_uring |

There is **no async socket code in the provider pipeline at all**:
`termy_x` (x_core) has zero tokio/reqwest/hyper dependencies
(`rg tokio crates/x_core` → no matches). All provider work is synchronous,
called from GPUI's `background_executor` (`x_panel/mod.rs`, ~20 spawn
sites), which is exactly the right shape for blocking calls. The only
tokio runtime in the app is the single-connection SQLite store above.

## 2. The only io_uring-shaped candidate, measured

The single place a ring could theoretically be inserted is draining the
OpenCLI child's stdout pipe (`io.rs:spawn_pipe_reader`: a std thread doing
an 8 KiB `read()` loop into an mpsc channel). Benchmarked on this box
(Linux 6.12.94 x86_64, rustc 1.99.1, `-O`), same read-loop pattern:

```
spawn+wait /bin/true            median ≈ 2–4 ms   (fork/exec — not an io_uring op)
read-loop  16 KiB               median ≈ 1 µs
read-loop 256 KiB               median ≈ 13–15 µs
read-loop   1 MiB               median ≈ 77–94 µs
read-loop   5 MiB (image cap)   median ≈ 0.6 ms
fs::read 4 KiB (config/cache)   ≈ 1.2 µs each
```

A realistic OpenCLI payload (420-tweet synthetic payload in
`crates/desktop_app/examples/perf_bench.rs`) is ~110 KB → drain cost
≈ 10 µs against a 5,700–25,000 ms call: **< 0.001 % of wall clock**. Even
abolishing the pipe read entirely — io_uring cannot do better than zero —
would be unmeasurable. What dominates instead, in order: Chrome page
automation inside OpenCLI (seconds), process spawn (ms), JSON parsing
(already attacked: zero-copy borrowed parse, −69 % allocations, commit
`b60335d`), and local filtering (already SIMD-attacked where it paid,
commit `8b5d10c`). None of those is a kernel I/O-completion problem.

## 3. Why the two candidate crates cannot fit anyway

- **tokio-uring 0.5.0** (May 2024, no release since; upstream issue
  tokio-rs/tokio-uring#336, Dec 2025: "there's no maintainer for this repo
  now"). Its `File`/`TcpStream` types are only usable inside
  `tokio_uring::start(...)` — a current-thread tokio runtime with an
  io_uring driver. Termy X's provider code does not run on tokio; it runs
  as blocking closures on GPUI's background executor. Adopting it means
  standing up a parallel runtime and bridging every provider call across
  it, to accelerate a 10 µs pipe drain inside a 10 s browser call. It also
  has no subprocess-pipe API, so the candidate in §2 isn't even expressible
  without hand-rolling spawn + raw ring code.
- **glommio** is a thread-per-core, single-threaded `LocalExecutor`
  (one per thread, ≥512 KiB memlock per executor, kernel ≥ 5.8). It is an
  application-architecture choice for dedicated I/O servers, not a library
  you drop into a GPUI event loop; its types are thread-local by design
  and cannot serve `Send + Sync` traits like `CommandRunner`/`Http`
  that are shared across GPUI background threads.
- Kernel availability is **not** the blocker: this box reports kernel
  6.12.94 and `/proc/sys/kernel/io_uring_disabled = 0`. The blocker is
  that there is no workload for it.

## 4. The correct Linux perf alternatives (already in flight / in tree)

1. **Keep std-thread blocking I/O on GPUI's background executor + epoll
   where the platform already uses it.** This is the current design and it
   is correct for subprocess + occasional blocking HTTPS. No change.
2. **mimalloc global allocator** (commit `b85ab19`, +20–25 % on
   allocation-heavy stages, zero API change) — the actually-measured win.
3. **Zero-copy borrowed parsing** (`RawPost<'a>` with `&'a str` fields,
   commit `b60335d`) and **single-pass/Aho-Corasick filtering where
   measured** (`8b5d10c`, reverted where slower) — CPU-side, where the
   profile says the time is.
4. **Attack the real seconds**: OpenCLI's own fixed sleeps and poll loops
   (`docs/OPENCLI-SPEED.md`, commit `1061fa9`). Another order of magnitude
   lives there, none of it in the kernel I/O path.

## 5. When this verdict should be revisited

Re-open only if the provider architecture changes shape, specifically:
(a) OpenCLI/Chrome is replaced by **direct in-process HTTPS to the X API**
with sustained high concurrency (hundreds+ in-flight sockets), or
(b) a local socket/IPC firehose (e.g. streaming API over a Unix socket)
becomes the ingest path. In case (a) the first move is still mainstream
**tokio (epoll) + rustls with connection pooling** — io_uring only
earns its complexity after a profiler shows syscall/completion overhead,
not network latency, as the bottleneck, and by then upstream tokio's own
uring support should be re-evaluated instead of the unmaintained
tokio-uring crate.
