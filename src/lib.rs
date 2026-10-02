pub mod capture;
pub mod commands;
pub mod connection;
pub mod context;
pub mod core;
pub mod demo;
pub mod files;
pub mod http_api;
pub mod mcp;
pub mod store;
pub mod study;

pub fn dispatch(
    store: &store::Store,
    name: &str,
    args: serde_json::Value,
) -> store::Result<serde_json::Value> {
    if name == "demo" {
        demo::seed(store)
    } else if core::handles(name) {
        core::call(store, name, args)
    } else if files::handles(name) {
        files::call(store, name, args)
    } else if commands::handles(name) {
        commands::call(store, name, args)
    } else if study::handles(name) {
        study::call(store, name, args)
    } else {
        Err(format!("Unknown tool: {name}"))
    }
}
