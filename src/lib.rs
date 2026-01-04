use clap::ValueEnum;

/// File extension/type for issue files.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum Extension {
	#[default]
	Md,
	Typ,
}

impl Extension {
	pub fn as_str(&self) -> &'static str {
		match self {
			Extension::Md => "md",
			Extension::Typ => "typ",
		}
	}
}
