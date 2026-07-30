fn main() {
    if let Err(error) = skiff_runtime_service_db::migration_tool::run_from_env() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
