# Release and Packaging

Harmonia currently supports three practical distribution paths.

## 1. Local install from source

```bash
cargo install --path .
```

Use this for local testing and development workflows.

## 2. Prebuilt tarball artifacts

Build static Linux artifacts with checksum:

```bash
VERSION=0.1.0
TARGET=x86_64-unknown-linux-musl
OUT_DIR=dist

build_static

mkdir -p "$OUT_DIR"
ARCHIVE_BASENAME="harmonia-${VERSION}-${TARGET}"
STAGE_DIR="$(mktemp -d)"
cp "target/${TARGET}/release/harmonia" "${STAGE_DIR}/harmonia"
tar -C "${STAGE_DIR}" -czf "${OUT_DIR}/${ARCHIVE_BASENAME}.tar.gz" harmonia
(
  cd "${OUT_DIR}"
  sha256sum "${ARCHIVE_BASENAME}.tar.gz" > "${ARCHIVE_BASENAME}.tar.gz.sha256"
)
rm -rf "${STAGE_DIR}"
```

Outputs:

- `dist/harmonia-0.1.0-x86_64-unknown-linux-musl.tar.gz`
- `dist/harmonia-0.1.0-x86_64-unknown-linux-musl.tar.gz.sha256`

Publish these on a release page for binary-first install workflows.


## Completion and Docs Artifacts

Before cutting a release, refresh generated assets:

```bash
mkdir -p docs/completions docs/cli
for shell in bash zsh fish elvish powershell; do
  harmonia completion "$shell" > "docs/completions/harmonia.$shell"
done

TOP_HELP="$(harmonia --help)"
printf "%s\n" "$TOP_HELP" > docs/cli/harmonia-help.txt
mapfile -t COMMANDS < <(
  printf "%s\n" "$TOP_HELP" | awk '/^Commands:$/ { in_commands = 1; next } in_commands && NF == 0 { exit } in_commands { print $1 }' | grep -v '^help$'
)
for cmd in "${COMMANDS[@]}"; do
  harmonia "$cmd" --help > "docs/cli/harmonia-${cmd}-help.txt"
done
```

This updates:

- `docs/completions/`
- `docs/cli/harmonia-*-help.txt`
