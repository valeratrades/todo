{ ...
}:
{
  name = "CI";
  permissions.contents = "read";
  jobs.pre_ci = {
    uses = "valeratrades/.github/.github/workflows/pre_ci.yml@master";
  };
  env = {
    CARGO_INCREMENTAL = "0"; # on large changes this just bloats ./target
    RUST_BACKTRACE = "short";
    CARGO_NET_RETRY = "10";
    RUSTUP_MAX_RETRIES = "10";
  };
}
