pub fn validate(input: &str) -> bool {
    !input.trim().is_empty() && input.len() < 256
}

pub fn process(input: &str) -> Result<String, String> {
    if !validate(input) {
        return Err("invalid or too long".to_string());
    }
    Ok(input.trim().to_uppercase())
}
