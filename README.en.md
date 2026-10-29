# nayuta

English | [日本語](README.md)

nayuta is a lightweight CLI for finding potentially expensive loops in Rust code. It also provides memory-access sampling on Linux through perf.

The static analyzer currently assumes loop iterations depend on input size and reports:

- Single loops as potential `O(N)` work
- Nested loops as potential `O(N^m)` work

## Usage

From a checkout:

```console
cargo run -- path/to/project
```

Without a path, nayuta recursively scans the current directory. It skips `target`, `.git`, symbolic links, and special files.

```console
# Exit with status 1 when findings are reported (useful in CI).
cargo run -- --fail src
```

Read and parse errors also cause a nonzero exit status. Analysis continues for other readable, valid files.

## Example output

```text
src/example.rs:12: complexity: loop may grow as O(N)
src/example.rs:18: complexity: loop nesting depth is 2; work may grow as O(N^2)
```

## Memory-access profiling on Linux

This feature requires Linux perf and a CPU and kernel that support memory sampling. Build the target with debug information to display source locations. For example, add `debug = true` under `[profile.release]` in the target project's Cargo.toml.

With nayuta installed:

```console
cargo build --release
nayuta memory --output run.data -- ./target/release/example argument
nayuta memory --input run.data
```

To run nayuta from its checkout instead, use `cargo run -- memory ...`.

nayuta runs the target through `perf mem record`, then uses `perf mem report` to display sample counts and distributions by source line and symbol. Reporting an existing recording also requires Linux and perf.

Existing output paths are rejected. On Unix, recording files are created with owner-only read/write permissions (`0600`, subject to umask). On failure, perf diagnostics are displayed and recording data is retained for investigation. If permissions prevent profiling, check the host's perf configuration.

These are samples, not exact counts of reads and writes. Supported events and sampling bias depend on the CPU; high-latency loads may be favored. Reported percentages are not cache-miss rates.

Custom aggregation by accessed memory region, cache-miss rate calculation, and growth estimation across input sizes are not implemented. Memory sampling on real hardware has not yet been validated for this project.

See the [perf mem documentation](https://manpages.debian.org/trixie/linux-perf/perf-mem.1.en.html) for backend details.

## Limitations

Static analysis uses heuristics based on Rust syntax trees. It does not distinguish fixed-count loops from input-dependent loops, so use findings as starting points for code review rather than proven complexity bounds.

Iterator chains, recursion, and complexity across function calls are not analyzed. Functions, methods, and supported constant-evaluation contexts are analyzed with independent loop depths.

## License

[MIT](LICENSE)
