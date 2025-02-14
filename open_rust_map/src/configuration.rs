use tokio::sync::OnceCell;

use self::setting::Settings;

pub mod setting;
pub static SETTINGS: OnceCell<Settings> = OnceCell::const_new();
