{ pkgs ? import
    (fetchTarball {
      name = "jpetrucciani-2026-01-21";
      url = "https://github.com/jpetrucciani/nix/archive/0fa40e09f3d6b7fe29811caef876444be9fa2a1a.tar.gz";
      sha256 = "16np1a2482l1s82yyxwh8d6igqqz4plc03fa9hv4mfricg2qicyi";
    })
    { overlays = [ _rust ]; }
, _rust ? import
    (fetchTarball {
      name = "oxalica-2026-01-21";
      url = "https://github.com/oxalica/rust-overlay/archive/2ef5b3362af585a83bafd34e7fc9b1f388c2e5e2.tar.gz";
      sha256 = "138a0p83qzflw8wj4a7cainqanjmvjlincx8imr3yq1b924lg9cz";
    })
}:
let
  name = "harmonia";

  target = "x86_64-unknown-linux-musl";
  rust = pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default.override {
    extensions = [ "rust-src" "rustc-dev" "rust-analyzer" ];
    targets = [ target ];
  });

  rustPlatform = pkgs.makeRustPlatform {
    cargo = rust;
    rustc = rust;
  };

  tools = with pkgs; {
    cli = [
      jfmt
    ];
    node = [ bun ];
    rust = [
      cargo-zigbuild
      rust
      pkg-config
    ];
    scripts = pkgs.lib.attrsets.attrValues scripts;
  };

  scripts = with pkgs; {
    build_static = writers.writeBashBin "build_static" ''
      cargo zigbuild --release --target "x86_64-unknown-linux-musl"
    '';
    generate_completions = writers.writeBashBin "generate_completions" ''
      set -eo pipefail

      repo_root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
      out_dir="$repo_root/docs/completions"
      if [ $# -ge 1 ]; then
        out_dir="$1"
      fi

      mkdir -p "$out_dir"

      for shell in bash zsh fish elvish powershell; do
        cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin harmonia -- completion "$shell" > "$out_dir/harmonia.$shell"
      done

      echo "generated completions in $out_dir"
    '';
    generate_docs = writers.writeBashBin "generate_docs" ''
      set -eo pipefail

      repo_root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
      out_dir="$repo_root/docs"
      if [ $# -ge 1 ]; then
        out_dir="$1"
      fi

      cli_dir="$out_dir/cli"
      man_dir="$out_dir/man"
      mkdir -p "$cli_dir" "$man_dir"

      run_help() {
        if [ -n "$HARMONIA_BIN" ]; then
          "$HARMONIA_BIN" "$@" --help
        else
          cargo run --quiet --manifest-path "$repo_root/Cargo.toml" --bin harmonia -- "$@" --help
        fi
      }

      top_help=$(run_help)
      printf "%s\n" "$top_help" > "$cli_dir/harmonia-help.txt"

      commands=$(printf "%s\n" "$top_help" | awk '
        /^Commands:$/ { in_commands = 1; next }
        in_commands && NF == 0 { exit }
        in_commands { print $1 }
      ' | grep -v '^help$' || true)

      for cmd in $commands; do
        run_help "$cmd" > "$cli_dir/harmonia-$cmd-help.txt"
      done

      version=$(awk -F'"' '/^version = / { print $2; exit }' "$repo_root/Cargo.toml")
      today=$(date +%Y-%m-%d)

      {
        echo ".TH HARMONIA 1 \"$today\" \"harmonia $version\" \"User Commands\""
        echo ".SH NAME"
        echo "harmonia \\- poly-repo orchestrator"
        echo ".SH SYNOPSIS"
        echo ".B harmonia"
        echo "[\\fIOPTIONS\\fR] <\\fICOMMAND\\fR>"
        echo ".SH DESCRIPTION"
        echo "Harmonia coordinates work across multiple repositories in one workspace."
        echo ".SH COMMANDS"
        for cmd in $commands; do
          echo ".TP"
          echo ".B $cmd"
        done
        echo ".SH COMPLETIONS"
        echo "Generate shell completions with:"
        echo ".IP"
        echo "harmonia completion bash"
        echo ".SH MORE HELP"
        echo "Detailed command help is generated in docs/cli/ as text snapshots."
      } > "$man_dir/harmonia.1"

      echo "generated docs in $out_dir"
    '';
    release_bundle = writers.writeBashBin "release_bundle" ''
      set -eo pipefail

      repo_root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)

      target="x86_64-unknown-linux-musl"
      if [ -n "$TARGET" ]; then
        target="$TARGET"
      fi

      if [ $# -ge 1 ]; then
        version="$1"
      else
        version=$(awk -F'"' '/^version = / { print $2; exit }' "$repo_root/Cargo.toml")
      fi

      out_dir="$repo_root/dist"
      if [ $# -ge 2 ]; then
        out_dir="$2"
      fi

      mkdir -p "$out_dir"

      cargo zigbuild --manifest-path "$repo_root/Cargo.toml" --release --target "$target"

      bin_path="$repo_root/target/$target/release/harmonia"
      archive_basename="harmonia-$version-$target"
      stage_dir=$(mktemp -d)

      cp "$bin_path" "$stage_dir/harmonia"
      tar -C "$stage_dir" -czf "$out_dir/$archive_basename.tar.gz" harmonia
      (
        cd "$out_dir"
        sha256sum "$archive_basename.tar.gz" > "$archive_basename.tar.gz.sha256"
      )
      rm -rf "$stage_dir"

      echo "created $out_dir/$archive_basename.tar.gz"
      echo "created $out_dir/$archive_basename.tar.gz.sha256"
    '';
    fixture_workspace = writers.writeBashBin "fixture_workspace" ''
            set -eo pipefail

            usage() {
              cat <<'EOF'
      Generate a local Harmonia fixture workspace with real git remotes and mixed ecosystems.

      Usage:
        fixture_workspace [OPTIONS] [OUTPUT_ROOT]

      Options:
        -f, --force    Remove OUTPUT_ROOT first if it already exists
        -h, --help     Show this help

      Outputs:
        <OUTPUT_ROOT>/workspace/.harmonia/config.toml
        <OUTPUT_ROOT>/workspace/.harmonia/config.with-cycle.toml
        <OUTPUT_ROOT>/remotes/*.git
      EOF
            }

            require_cmd() {
              if ! command -v "$1" >/dev/null 2>&1; then
                echo "missing required command: $1" >&2
                exit 1
              fi
            }

            write_shared_lib_repo() {
              dir="$1"
              mkdir -p "$dir/src"
              cat > "$dir/Cargo.toml" <<'EOF'
      [package]
      name = "shared-lib"
      version = "0.1.0"
      edition = "2021"

      [dependencies]
      EOF
              cat > "$dir/src/lib.rs" <<'EOF'
      pub fn shared_value() -> &'static str {
          "shared-lib"
      }
      EOF
              cat > "$dir/.harmonia.toml" <<'EOF'
      [package]
      name = "shared-lib"
      ecosystem = "rust"

      [dependencies]
      file = "Cargo.toml"

      [hooks.custom]
      test = "echo shared-lib-test"
      lint = "echo shared-lib-lint"
      EOF
            }

            write_service_a_repo() {
              dir="$1"
              mkdir -p "$dir/service_a"
              cat > "$dir/pyproject.toml" <<'EOF'
      [project]
      name = "service-a"
      version = "0.1.0"
      dependencies = [
        "shared-lib>=0.1.0",
      ]
      EOF
              cat > "$dir/service_a/__init__.py" <<'EOF'
      def service_name() -> str:
          return "service-a"
      EOF
              cat > "$dir/.harmonia.toml" <<'EOF'
      [package]
      name = "service-a"
      ecosystem = "python"

      [dependencies]
      file = "pyproject.toml"

      [hooks.custom]
      test = "echo service-a-test"
      lint = "echo service-a-lint"
      EOF
            }

            write_web_ui_repo() {
              dir="$1"
              mkdir -p "$dir/src"
              cat > "$dir/package.json" <<'EOF'
      {
        "name": "web-ui",
        "version": "0.1.0",
        "dependencies": {
          "shared-lib": "^0.1.0"
        }
      }
      EOF
              cat > "$dir/src/index.js" <<'EOF'
      console.log("web-ui");
      EOF
              cat > "$dir/.harmonia.toml" <<'EOF'
      [package]
      name = "web-ui"
      ecosystem = "node"

      [dependencies]
      file = "package.json"

      [hooks.custom]
      test = "echo web-ui-test"
      lint = "echo web-ui-lint"
      EOF
            }

            write_gateway_repo() {
              dir="$1"
              cat > "$dir/go.mod" <<'EOF'
      module gateway

      go 1.22.0

      require (
        service-a v0.1.0
        web-ui v0.1.0
      )
      EOF
              cat > "$dir/.harmonia.toml" <<'EOF'
      [package]
      name = "gateway"
      ecosystem = "go"

      [dependencies]
      file = "go.mod"

      [hooks.custom]
      test = "echo gateway-test"
      lint = "echo gateway-lint"
      EOF
            }

            write_external_sdk_repo() {
              dir="$1"
              cat > "$dir/package.json" <<'EOF'
      {
        "name": "external-sdk",
        "version": "1.0.0"
      }
      EOF
              cat > "$dir/.harmonia.toml" <<'EOF'
      [package]
      name = "external-sdk"
      ecosystem = "node"
      EOF
            }

            write_scratch_repo() {
              dir="$1"
              cat > "$dir/go.mod" <<'EOF'
      module scratch

      go 1.22.0
      EOF
              cat > "$dir/.harmonia.toml" <<'EOF'
      [package]
      name = "scratch"
      ecosystem = "go"
      EOF
            }

            write_cycle_a_repo() {
              dir="$1"
              mkdir -p "$dir/src"
              cat > "$dir/Cargo.toml" <<'EOF'
      [package]
      name = "cycle-a"
      version = "0.1.0"
      edition = "2021"

      [dependencies]
      cycle-b = "0.1.0"
      EOF
              cat > "$dir/src/lib.rs" <<'EOF'
      pub fn cycle_a() -> &'static str {
          "cycle-a"
      }
      EOF
              cat > "$dir/.harmonia.toml" <<'EOF'
      [package]
      name = "cycle-a"
      ecosystem = "rust"

      [dependencies]
      file = "Cargo.toml"
      EOF
            }

            write_cycle_b_repo() {
              dir="$1"
              mkdir -p "$dir/src"
              cat > "$dir/Cargo.toml" <<'EOF'
      [package]
      name = "cycle-b"
      version = "0.1.0"
      edition = "2021"

      [dependencies]
      cycle-a = "0.1.0"
      EOF
              cat > "$dir/src/lib.rs" <<'EOF'
      pub fn cycle_b() -> &'static str {
          "cycle-b"
      }
      EOF
              cat > "$dir/.harmonia.toml" <<'EOF'
      [package]
      name = "cycle-b"
      ecosystem = "rust"

      [dependencies]
      file = "Cargo.toml"
      EOF
            }

            write_repo() {
              name="$1"
              dir="$2"
              case "$name" in
                shared-lib) write_shared_lib_repo "$dir" ;;
                service-a) write_service_a_repo "$dir" ;;
                web-ui) write_web_ui_repo "$dir" ;;
                gateway) write_gateway_repo "$dir" ;;
                external-sdk) write_external_sdk_repo "$dir" ;;
                scratch) write_scratch_repo "$dir" ;;
                cycle-a) write_cycle_a_repo "$dir" ;;
                cycle-b) write_cycle_b_repo "$dir" ;;
                *)
                  echo "unknown fixture repo: $name" >&2
                  exit 1
                  ;;
              esac
            }

            seed_remote_repo() {
              name="$1"
              bare="$remotes_dir/$name.git"
              seed="$seed_dir/$name"

              git init --bare --quiet "$bare"
              git init --quiet "$seed"
              git -C "$seed" config user.name "Harmonia Fixture"
              git -C "$seed" config user.email "harmonia-fixture@example.com"

              write_repo "$name" "$seed"

              git -C "$seed" add -A
              git -C "$seed" commit --quiet -m "Initial fixture commit"
              git -C "$seed" branch -M main
              git -C "$seed" remote add origin "$bare"
              git -C "$seed" push --quiet -u origin main
            }

            repo_url() {
              name="$1"
              printf 'file://%s/%s.git' "$remotes_dir" "$name"
            }

            write_workspace_config() {
              config_path="$1"
              include_cycles="$2"

              cat > "$config_path" <<EOF
      [workspace]
      name = "harmonia-local-fixture"
      repos_dir = "repos"

      [repos]
      "shared-lib" = { url = "$(repo_url shared-lib)" }
      "service-a" = { url = "$(repo_url service-a)" }
      "web-ui" = { url = "$(repo_url web-ui)" }
      "gateway" = { url = "$(repo_url gateway)" }
      "external-sdk" = { url = "$(repo_url external-sdk)", external = true }
      "scratch" = { url = "$(repo_url scratch)", ignored = true }
      EOF

              if [ "$include_cycles" = "yes" ]; then
                cat >> "$config_path" <<EOF
      "cycle-a" = { url = "$(repo_url cycle-a)" }
      "cycle-b" = { url = "$(repo_url cycle-b)" }
      EOF
              fi

              cat >> "$config_path" <<'EOF'

      [groups]
      core = ["shared-lib", "service-a", "web-ui", "gateway"]
      explore = ["shared-lib", "service-a", "web-ui", "gateway", "external-sdk"]
      default = "core"
      EOF

              if [ "$include_cycles" = "yes" ]; then
                cat >> "$config_path" <<'EOF'
      cycle = ["cycle-a", "cycle-b"]
      EOF
              fi

              cat >> "$config_path" <<'EOF'

      [defaults]
      default_branch = "main"
      clone_protocol = "https"
      clone_depth = "full"
      include_untracked = true

      [hooks.custom]
      test = "echo workspace-test"
      lint = "echo workspace-lint"

      [versioning]
      strategy = "semver"
      bump_mode = "semver"
      cascade_bumps = false
      EOF
            }

            tmp_root="$TMPDIR"
            if [ -z "$tmp_root" ]; then
              tmp_root="/tmp"
            fi
            output_root="$tmp_root/harmonia-local-fixture"
            force=0
            output_arg_set=0

            while [ $# -gt 0 ]; do
              case "$1" in
                -f|--force)
                  force=1
                  shift
                  ;;
                -h|--help)
                  usage
                  exit 0
                  ;;
                *)
                  if [ "$output_arg_set" -eq 1 ]; then
                    echo "unexpected argument: $1" >&2
                    usage
                    exit 2
                  fi
                  output_root="$1"
                  output_arg_set=1
                  shift
                  ;;
              esac
            done

            case "$output_root" in
              *" "*)
                echo "output path cannot contain spaces: $output_root" >&2
                exit 1
                ;;
            esac

            require_cmd git

            if [ -e "$output_root" ]; then
              if [ "$force" -eq 1 ]; then
                rm -rf "$output_root"
              else
                echo "output root already exists: $output_root" >&2
                echo "use --force to overwrite" >&2
                exit 1
              fi
            fi

            remotes_dir="$output_root/remotes"
            workspace_dir="$output_root/workspace"
            seed_dir="$output_root/.seed"

            mkdir -p "$remotes_dir" "$workspace_dir/.harmonia" "$seed_dir"

            for repo in \
              shared-lib \
              service-a \
              web-ui \
              gateway \
              external-sdk \
              scratch \
              cycle-a \
              cycle-b
            do
              seed_remote_repo "$repo"
            done

            write_workspace_config "$workspace_dir/.harmonia/config.toml" "no"
            write_workspace_config "$workspace_dir/.harmonia/config.with-cycle.toml" "yes"

            rm -rf "$seed_dir"

            cat > "$workspace_dir/README.fixture.md" <<'EOF'
      # Harmonia Local Fixture Workspace

      Primary config (acyclic):
      - `.harmonia/config.toml`

      Cycle config (includes cycle-a <-> cycle-b):
      - `.harmonia/config.with-cycle.toml`

      Suggested commands:
      - `harmonia clone --all`
      - `harmonia graph show --format=tree`
      - `harmonia graph order --json`
      - `harmonia test --all`
      - `harmonia lint --all`
      EOF

            echo "fixture generated:"
            echo "  remotes:   $remotes_dir"
            echo "  workspace: $workspace_dir"
            echo "  config:    $workspace_dir/.harmonia/config.toml"
            echo "  cycle cfg: $workspace_dir/.harmonia/config.with-cycle.toml"
            echo
            echo "next:"
            echo "  smoke_fixture \"$workspace_dir\""
    '';
    smoke_fixture = writers.writeBashBin "smoke_fixture" ''
            set -eo pipefail

            usage() {
              cat <<'EOF'
      Run a smoke flow against a local Harmonia workspace fixture.

      Usage:
        smoke_fixture <WORKSPACE_PATH>

      Environment:
        HARMONIA_BIN   Optional path to a prebuilt harmonia binary.
                       If unset, the script runs `cargo run` from this repo.
      EOF
            }

            if [ $# -ne 1 ]; then
              usage
              exit 2
            fi

            workspace="$1"
            if [ ! -d "$workspace/.harmonia" ]; then
              echo "workspace missing .harmonia: $workspace" >&2
              exit 1
            fi

            repo_root=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
            main_config="$workspace/.harmonia/config.toml"
            cycle_config="$workspace/.harmonia/config.with-cycle.toml"
            init_sanity_dir="$workspace/.smoke-init"

            step() {
              echo "==> $*" >&2
            }

            run_harmonia() {
              if [ -n "$HARMONIA_BIN" ]; then
                "$HARMONIA_BIN" "$@"
              else
                cargo run --quiet --manifest-path "$repo_root/Cargo.toml" -- "$@"
              fi
            }

            if [ ! -f "$main_config" ]; then
              echo "missing config: $main_config" >&2
              exit 1
            fi

            step "init sanity check"
            rm -rf "$init_sanity_dir"
            run_harmonia init --name smoke-init --no-clone --directory "$init_sanity_dir" >/dev/null
            rm -rf "$init_sanity_dir"

            step "clone all repos"
            run_harmonia --workspace "$workspace" clone --all

            step "sync"
            run_harmonia --workspace "$workspace" sync

            step "status json"
            run_harmonia --workspace "$workspace" status --json >/dev/null

            step "graph show/deps/dependents/order/check"
            run_harmonia --workspace "$workspace" graph show --format=tree >/dev/null
            run_harmonia --workspace "$workspace" graph deps gateway >/dev/null
            run_harmonia --workspace "$workspace" graph dependents shared-lib >/dev/null
            run_harmonia --workspace "$workspace" graph order --json >/dev/null
            run_harmonia --workspace "$workspace" graph check --json >/dev/null

            step "version show/check/bump dry-run"
            run_harmonia --workspace "$workspace" version show --json >/dev/null
            run_harmonia --workspace "$workspace" version check --json >/dev/null
            run_harmonia --workspace "$workspace" version bump patch --repos shared-lib --dry-run >/dev/null

            step "deps show/check/update dry-run"
            run_harmonia --workspace "$workspace" deps show --json >/dev/null
            run_harmonia --workspace "$workspace" deps check --json >/dev/null
            run_harmonia --workspace "$workspace" deps update shared-lib --dry-run >/dev/null

            step "mark one repo as changed and run test/lint"
            printf '# smoke change\n' >> "$workspace/repos/service-a/SMOKE.md"
            run_harmonia --workspace "$workspace" test --changed -k smoke >/dev/null
            run_harmonia --workspace "$workspace" lint --changed --parallel 2 >/dev/null

            if [ -f "$cycle_config" ]; then
              step "clone cycle repos"
              run_harmonia --config "$cycle_config" clone cycle-a cycle-b >/dev/null

              step "cycle config graph check"
              run_harmonia --config "$cycle_config" graph check --json >/dev/null

              step "cycle config graph order should fail"
              if run_harmonia --config "$cycle_config" graph order --json >/dev/null 2>&1; then
                echo "expected graph order to fail with cycle config, but it succeeded" >&2
                exit 1
              fi
            fi

            echo "smoke complete: $workspace"
    '';
  };
  paths = pkgs.lib.flatten [ (builtins.attrValues tools) ];
  env = pkgs.buildEnv {
    inherit name paths; buildInputs = paths;
  };
  bin = rustPlatform.buildRustPackage (finalAttrs: {
    pname = name;
    version = "0.0.0";
    src = pkgs.hax.filterSrc { path = ./.; };
    cargoLock.lockFile = ./Cargo.lock;
    auditable = false;
    nativeBuildInputs = with pkgs; [
      cargo-zigbuild
    ];
    buildPhase = ''
      export HOME=$(mktemp -d)
      cargo zigbuild --release --target ${target}
    '';
    installPhase = ''
      mkdir -p $out/bin
      cp target/${target}/release/${name} $out/bin/
    '';
  });

in
(env.overrideAttrs (_: {
  inherit name;
  NIXUP = "0.0.10";
})) // { inherit bin scripts; }
