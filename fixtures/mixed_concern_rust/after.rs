pub struct Counter {
    value: i32,
}

impl Counter {
    pub fn new() -> Self {
        Counter { value: 0 }
    }

    pub fn increment(&mut self) {
        self.value += 2;
    }

    pub fn value(&self) -> i32 {
        self.value
    }
}

pub fn format_report(title: &str) -> String {
    format!("== Report: {title} ==")
}

pub fn parse_config(input: &str) -> Vec<String> {
    input.split(',').map(|s| s.trim().to_string()).collect()
}
