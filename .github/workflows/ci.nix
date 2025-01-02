{ pkgs
, workflow-parts
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

  #imports = [ workflow-parts.shared { inherit pkgs; } ];

  jobs = {
    tokei = import workflow-parts.tokei { inherit pkgs; };
    tests = import workflow-parts.tests { inherit pkgs; };
    doc = import workflow-parts.doc { inherit pkgs; };
    miri = import workflow-parts.miri { inherit pkgs; };
    clippy = import workflow-parts.clippy { inherit pkgs; };
    machete = import workflow-parts.machete { inherit pkgs; };
    sort = import workflow-parts.sort { inherit pkgs; };
  };

  #env = {
  #  RUSTFLAGS = "-Dwarnings";
  #};
}
