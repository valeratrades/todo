{ ...
}: {
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
}
