{ ...
}: {
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
}
