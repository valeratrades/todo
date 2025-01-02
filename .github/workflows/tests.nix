{ ...
}: {
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
      run = "echo RUSTFLAGS=\${RUSTFLAGS}\\ -Zrandomize-layout\\ --cfg=exhaustive >> $GITHUB_ENV";
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
}
