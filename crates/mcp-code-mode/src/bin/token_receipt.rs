fn main() {
    let text = mcp_code_mode::render_receipt();
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "RECEIPT.md".to_string());
    if let Err(error) = std::fs::write(&path, &text) {
        eprintln!("token-receipt: failed to write {path}: {error}");
        std::process::exit(1);
    }
    print!("{text}");
}
