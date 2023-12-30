fn main() {
	let mut prev_activity = String::new();
	loop {
		let activity = get_activity();
		if prev_activity != activity {
			println!("{}", activity);
			prev_activity = activity;
		}
		std::thread::sleep(std::time::Duration::from_secs(1));
	}
}

fn get_activity() -> String {
	let output = std::process::Command::new("sh")
		.arg("-c")
		.arg("swaymsg -t get_workspaces | jq -r '.[] | select(.focused==true).name'")
		.output()
		.unwrap();
	let output_str = String::from_utf8_lossy(&output.stdout);
	let focused = output_str.trim_end_matches('\n');

	//if focused == "2".to_owned()

	let activity: String = match focused.parse() {
		Ok(1) => "neovim".to_owned(),
		//TODO!!!!: get the tab
		Ok(2) => "googling".to_owned(),
		Ok(3) => "editing todos/notes".to_owned(),
		//TODO!: get which one
		Ok(4) => "social networks".to_owned(),
		Ok(5) => "reading a book".to_owned(),
		Ok(num) => format!("ws{}", num),
		Err(_) => unreachable!(),
	};

	activity
}
