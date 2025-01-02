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
    tokei = import ../tokei.nix;

    pre_ci = {
      uses = "valeratrades/.github/.github/workflows/pre_ci.yml@master";
    };

    tests = import ../test.nix;

    doc = import ../doc.nix;

    miri = import ../miri.nix;

    clippy = import ../clippy.nix;

    machete = import ../machete.nix;

    sort = import ../sort.nix;
  };

  env = {
    #RUSTFLAGS = "-Dwarnings";
    CARGO_INCREMENTAL = "0"; # on large changes this just bloats ./target
    RUST_BACKTRACE = "short";
    CARGO_NET_RETRY = "10";
    RUSTUP_MAX_RETRIES = "10";
  };
}
