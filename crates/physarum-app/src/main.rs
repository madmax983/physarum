// Physarum — a slime mold grown in a SQLite fleet split between hosts.
//
//   physarum-app        the console (http://localhost:3000): starts host-a
//                          and host-b as child processes in ./physarum-lab,
//                          grows the mold through them, kills and freezes them
//                          on request.
//   physarum-app host   one host (the console runs these): a tenant fleet,
//                          one tenant per cell of the dish, with ownership and
//                          replication. Reads autumn.toml from its directory.

use autumn_web::migrate::{EmbeddedMigrations, embed_migrations};
use autumn_web::prelude::*;
use physarum_app::console::{self, routes};
use physarum_app::host;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

#[autumn_web::main]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("host") {
        Box::pin(
            autumn_web::app()
                .migrations(MIGRATIONS)
                .routes(routes![host::read, host::write])
                .run(),
        )
        .await;
    } else {
        Box::pin(
            autumn_web::app()
                .on_startup(console::boot)
                .routes(routes![
                    routes::index,
                    routes::frame,
                    routes::lab_state,
                    routes::host_action,
                    routes::mold,
                    routes::food,
                    routes::clear_food,
                    routes::reset,
                    routes::script,
                ])
                .run(),
        )
        .await;
    }
}
