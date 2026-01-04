//! Test-only planner stub that echoes a deterministic probe plan.

use serde_json::json;
use serde_json::Value;
use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("failed to read planner request");
        std::process::exit(1);
    }
    let request: Value = match serde_json::from_str(&input) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("failed to parse planner request: {err}");
            std::process::exit(1);
        }
    };

    let options = request
        .get("options")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let max_per_option = request
        .get("budget")
        .and_then(|value| value.get("max_per_option"))
        .and_then(|value| value.as_u64())
        .unwrap_or(1);
    let max_total = request
        .get("budget")
        .and_then(|value| value.get("max_total"))
        .and_then(|value| value.as_u64())
        .unwrap_or(options.len() as u64);

    let mut plan_options = Vec::new();
    let mut total_probes = 0u64;

    for option in options {
        let Some(option_name) = option.as_str() else {
            continue;
        };
        let mut probes = vec!["existence"];
        if max_per_option >= 2 {
            probes.push("invalid_value");
        }
        if max_per_option >= 3 {
            probes.push("option_at_end");
        }
        total_probes += probes.len() as u64;
        if total_probes > max_total {
            break;
        }
        plan_options.push(json!({
            "option": option_name,
            "probes": probes,
        }));
    }

    let budget = request.get("budget").cloned().unwrap_or_else(|| {
        json!({
            "max_total": max_total,
            "max_per_option": max_per_option,
        })
    });
    let stop_rules = request.get("stop_rules").cloned().unwrap_or_else(|| {
        json!({
            "stop_on_unrecognized": true,
            "stop_on_binding_confirmed": true,
        })
    });

    let plan = json!({
        "planner_version": "stub-v1",
        "options": plan_options,
        "budget": budget,
        "stop_rules": stop_rules,
    });

    println!("{}", plan);
}
