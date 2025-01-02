{ pkgs
, ...
}:
(pkgs.formats.yaml { }).generate "" {
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

  (import ../shared.nix);

  jobs = {
    tokei = import ../tokei.nix;

    tests = import ../test.nix;

    doc = import ../doc.nix;

    miri = import ../miri.nix;

    clippy = import ../clippy.nix;

    machete = import ../machete.nix;

    sort = import ../sort.nix;
  };

  #env = {
  #  RUSTFLAGS = "-Dwarnings";
  #};
}
