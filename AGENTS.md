# Repository Guide

## Updating Proto Definitions From BSR

This repository generates Rust protobuf/tonic code from the BSR module configured in `buf.gen.yaml` (`buf.build/voron-software/rafn`). To pull the latest published definitions:

1. Make sure `buf` is installed and authenticated if the BSR module or plugins require it:
   `buf registry login`.
2. From the repository root, run `buf generate`.
3. Check `src/proto/gen/` for generated file name changes. If the proto package changed, update the include in `src/proto/mod.rs` and remove stale generated files that `buf generate` left behind.
4. Run `cargo check --all-targets` to verify the generated API still matches the Rust code.
5. Review `git diff` before committing; generated files are checked in.
