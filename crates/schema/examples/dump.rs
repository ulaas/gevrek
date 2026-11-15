//! Print the configuration's JSON Schema: `cargo run -p gevrek-schema --example dump`.
fn main() {
    let text =
        serde_json::to_string_pretty(gevrek_schema::schema().document()).expect("a schema is JSON");
    println!("{text}");
}
