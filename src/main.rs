mod app;
mod changeset;
mod config_store;
mod forge_cfg;
mod profiles;
mod search;
mod settings;

pub fn main() -> iced::Result {
    iced::application(app::boot, app::update, app::view).title("GTNH Config Manager").run()
}

