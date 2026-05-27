# Rafn

Lightweight benchmark uploader

## GitHub Actions

Use this repository as a GitHub Action to install the `rafn` CLI in a workflow:

```yaml
steps:
  - uses: voron-software/rafn@v0.1.0
    with:
      version: v0.1.0

  - run: rafn --help
```

Set `version: latest` to download the newest GitHub Release. Release assets are expected to use the same names as the npm package binaries: `rafn-linux-x64`, `rafn-linux-arm64`, `rafn-darwin-x64`, `rafn-darwin-arm64`, and `rafn-win32-x64.exe`.
