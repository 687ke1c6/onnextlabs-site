// `sqlx::migrate!` embeds `migrations/` at compile time, but cargo doesn't know to recompile when a
// migration is added or changed: without this, a binary built earlier silently runs (and records)
// an older set of migrations against the database.
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}
