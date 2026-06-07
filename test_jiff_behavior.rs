fn main() {
    println!("Testing jiff::Timestamp parsing RFC3339 strictness\n");
    
    let test_cases = vec![
        ("2026-06-07T12:00:00Z", "with Z"),
        ("2026-06-07T12:00:00-05:00", "with explicit offset"),
        ("2026-06-07T12:00:00", "bare local datetime - NO TIMEZONE"),
        ("2026-06-07", "bare date only"),
        ("yesterday", "junk string"),
    ];
    
    for (input, desc) in test_cases {
        match input.parse::<jiff::Timestamp>() {
            Ok(ts) => println!("✓ ACCEPTS '{}' ({})\n  Parsed as: {:?}", input, desc, ts),
            Err(e) => println!("✗ REJECTS '{}' ({})\n  Error: {}", input, desc, e),
        }
        println!();
    }
}
