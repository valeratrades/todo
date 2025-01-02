{ pkgs
, ...
}:
(pkgs.formats.yaml { }).generate "" {
  name = "CI";

  on = {
    push = { };
    pull_request = { };
    workflow_dispatch = { };
    schedule = [
      {
        cron = "0 0 1 * *"; # Runs at midnight on the 1st day of every month
      }
    ];
  };

  permissions.contents = "read";

  jobs = {
    tokei = {
      name = "Reasonable Amount of Comments";
      runs-on = "ubuntu-latest";
      steps = [
        {
          name = "Checkout repository";
          uses = "actions/checkout@v4";
        }
        {
          name = "Installation";
          uses = "taiki-e/install-action@v2";
          "with".tool = "tokei";
        }
        {
          name = "Generate Tokei output";
          run = "tokei -o json > tokei_output.json";
        }
        {
          name = "Install jq";
          run = "sudo apt-get install -y jq";
        }
        {
          name = "Check Rust comments";
          # TODO: Generalize to other languages. Dynamically determine the most used language in the repo (excluding markdown, Jupyter, etc.).
          run = ''
            comments=$(jq '.Rust.comments' tokei_output.json)
            code=$(jq '.Rust.code' tokei_output.json)
            if [ $((comments * 10)) -ge $code ]; then
              echo "Number of comments should be less than 10% of code"
              exit 1
            else
              echo "Check passed: Number of comments is less than 10% of code"
            fi
          '';
        }
      ];
    };

    pre_ci = {
      uses = "valeratrades/.github/.github/workflows/pre_ci.yml@master";
    };

    test = {
      name = "Rust \${{matrix.rust}}";
      needs = "pre_ci";
      "if" = "needs.pre_ci.outputs.continue";
      runs-on = "ubuntu-latest";
      strategy = {
        fail-fast = false;
        matrix.rust = [ "nightly" "stable" ]; # dtolnay had [nightly, beta, stable, 1.70.0], hence the matrix
      };
      timeout-minutes = 45;
      steps = [
        {
          uses = "actions/checkout@v4";
        }
        {
          uses = "dtolnay/rust-toolchain@master";
          "with".toolchain = "\${{matrix.rust}}";
        }
        {
          # test this works
          name = "Set RUSTFLAGS for release branch";
          run = "echo \"RUSTFLAGS=-Dwarnings\" >> $GITHUB_ENV";
          "if" = "github.ref == 'refs/heads/release'";
        }
        {
          name = "Enable type layout randomization";
          run = "echo RUSTFLAGS=${RUSTFLAGS}\\ -Zrandomize-layout\\ --cfg=exhaustive >> $GITHUB_ENV";
          "if" = "matrix.rust == 'nightly'";
        }
        # not sure why dtolnay has this
        #{ run = "cargo check --locked"; }
        { run = "cargo update"; }
        { run = "cargo check"; }
        { run = "cargo test"; }
        #TODO: figure this out
        #  if: matrix.os == 'ubuntu' && matrix.rust == 'nightly'
        #- run: cargo run -- expand --manifest-path tests/Cargo.toml > expand.rs && diff tests/lib.expand.rs expand.rs
      ];
    };

    doc = {
      name = "Documentation";
      needs = "pre_ci";
      "if" = "needs.pre_ci.outputs.continue";
      runs-on = "ubuntu-latest";
      timeout-minutes = 45;
      env.RUSTDOCFLAGS = "-Dwarnings";
      steps = [
        { uses = "actions/checkout@v4"; }
        { uses = "dtolnay/rust-toolchain@nightly"; }
        { uses = "dtolnay/install@cargo-docs-rs"; }
        { run = "cargo docs-rs"; }
      ];
    };

    miri = {
      name = "Miri";
      needs = "pre_ci";
      "if" = "needs.pre_ci.outputs.continue";
      runs-on = "ubuntu-latest";
      timeout-minutes = 45;
      steps = [
        { uses = "actions/checkout@v4"; }
        { uses = "dtolnay/rust-toolchain@miri"; }
        { run = "cargo miri setup"; }
        {
          run = "cargo miri test";
          env.MIRIFLAGS = "-Zmiri-strict-provenance";
        }
      ];
    };

    clippy = {
      name = "Clippy";
      runs-on = "ubuntu-latest";
      "if" = "github.event_name != 'pull_request'";
      timeout-minutes = 45;
      steps = [
        { uses = "actions/checkout@v4"; }
        { uses = "dtolnay/rust-toolchain@clippy"; }
        { run = "cargo clippy --tests -- -Dwarnings"; } #-Dclippy::all #-Dclippy::pedantic
      ];
    };

    # they have their own GHA, but it uses `cargo install`. Until they transfer to binstall, this is better.
    machete = {
      name = "Unused Dependencies";
      runs-on = "ubuntu-latest";
      steps = [
        {
          name = "Installation";
          uses = "taiki-e/install-action@v2";
          "with".tool = "cargo-machete";
        }
        {
          name = "Cargo Machete";
          run = ''
            cargo machete
            exit_code=$?
            if [ $exit_code = 0 ]; then
              echo "Found unused dependencies"
              exit $exit_code
            fi
          '';
        }
      ];
    };

    sort = {
      name = "Cargo Sorted";
      runs-on = "ubuntu-latest";
      steps = [
        { uses = "actions/checkout@v4"; }
        {
          name = "Installation";
          uses = "taiki-e/install-action@v2";
          "with".tool = "cargo-sort";
        }
        {
          name = "Check if Cargo.toml is sorted";
          run = ''
            cargo sort -wc
            exit_code=$?
            if [ $exit_code != 0 ]; then
              echo "Cargo.toml is not sorted. Run \`cargo sort -w\` to fix it."
              exit $exit_code
            fi
          '';
        }
      ];
    };
  };

  env = {
    #RUSTFLAGS = "-Dwarnings";
    CARGO_INCREMENTAL = "0"; # on large changes this just bloats ./target
    RUST_BACKTRACE = "short";
    CARGO_NET_RETRY = "10";
    RUSTUP_MAX_RETRIES = "10";
  };
}
