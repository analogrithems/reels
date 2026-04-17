//! `reel` binary entry — initializes logging and runs [`reel_app::run`].

fn main() -> anyhow::Result<()> {
    reel_app::run()
}
