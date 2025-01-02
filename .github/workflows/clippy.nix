{ ... }: {
  name = "Clippy";
  runs-on = "ubuntu-latest";
  "if" = "github.event_name != 'pull_request'";
  timeout-minutes = 45;
  steps = [
    { uses = "actions/checkout@v4"; }
    { uses = "dtolnay/rust-toolchain@clippy"; }
    { run = "cargo clippy --tests -- -Dwarnings"; } #-Dclippy::all #-Dclippy::pedantic
  ];
}
