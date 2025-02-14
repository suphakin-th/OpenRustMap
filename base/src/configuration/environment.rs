use crate::error;

pub enum Environment {
	Local,
	Develop,
	Production,
}

impl Environment {
	pub fn as_str(&self) -> &'static str {
		match self {
			Environment::Local => "local",
			Environment::Develop => "develop",
			Environment::Production => "production",
		}
	}
}

impl TryFrom<String> for Environment {
	type Error = error::Error;

	fn try_from(s: String) -> Result<Self, Self::Error> {
		match s.to_lowercase().as_str() {
			"local" => Ok(Self::Local),
			"develop" => Ok(Self::Develop),
			"production" => Ok(Self::Production),
			_ => Err(error::Error::UnsupportEnv),
		}
	}
}