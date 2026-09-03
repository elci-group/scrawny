pub fn validate(input: &str) -> bool {
    !input.is_empty()
}

pub fn process(input: &str) -> Result<String, String> {
    if !validate(input) {
        return Err("invalid".to_string());
    }
    Ok(input.to_uppercase())
}
