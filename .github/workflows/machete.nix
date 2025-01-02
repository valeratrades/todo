{ ... }: {
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
      # they have their own GHA, but it uses `cargo install`. Until they transfer to binstall, this is better.
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
}
