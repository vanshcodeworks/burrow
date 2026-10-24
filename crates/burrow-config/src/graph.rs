//! Dependency graph construction and cycle reporting.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::ConfigError;
use crate::model::Config;

/// A deterministic, layered startup plan.
///
/// Services in the same layer have no dependency between them and may be
/// started concurrently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupPlan {
    /// Topological layers. Every layer is sorted by service name.
    pub layers: Vec<Vec<String>>,
    /// Services included in the plan, including transitive dependencies.
    pub included: BTreeSet<String>,
}

impl StartupPlan {
    /// Services in dependency-first order.
    pub fn order(&self) -> Vec<String> {
        self.layers.iter().flatten().cloned().collect()
    }
}

pub(crate) fn build(config: &Config, selected: &[String]) -> Result<StartupPlan, ConfigError> {
    let included = config.selected_services(selected)?;
    let mut indegree: BTreeMap<String, usize> = included
        .iter()
        .map(|name| {
            let dependencies = config.services[name]
                .needs
                .iter()
                .filter(|dependency| included.contains(*dependency))
                .count();
            (name.clone(), dependencies)
        })
        .collect();

    let mut layers = Vec::new();
    let mut remaining: BTreeSet<String> = included.clone();

    while !remaining.is_empty() {
        let layer: Vec<String> = remaining
            .iter()
            .filter(|name| indegree.get(*name) == Some(&0))
            .cloned()
            .collect();

        if layer.is_empty() {
            let cycle = find_cycle(config, &remaining)
                .unwrap_or_else(|| remaining.iter().cloned().collect::<Vec<String>>());
            return Err(ConfigError::Cycle {
                path: cycle.join(" -> "),
            });
        }

        for name in &layer {
            remaining.remove(name);
            indegree.remove(name);
            for (candidate, service) in &config.services {
                if included.contains(candidate) && service.needs.iter().any(|need| need == name) {
                    if let Some(value) = indegree.get_mut(candidate) {
                        *value = value.saturating_sub(1);
                    }
                }
            }
        }

        layers.push(layer);
    }

    Ok(StartupPlan { layers, included })
}

fn find_cycle(config: &Config, remaining: &BTreeSet<String>) -> Option<Vec<String>> {
    let mut state: BTreeMap<&str, u8> = BTreeMap::new();
    let mut stack: Vec<&str> = Vec::new();

    for name in remaining {
        if visit(
            config,
            remaining,
            name,
            &mut state,
            &mut stack,
            &mut Vec::new(),
        )
        .is_some()
        {
            // The out-parameter above is only used to keep the signature simple;
            // recompute with a visible path below.
            return cycle_from(config, remaining, name);
        }
    }
    None
}

fn visit<'a>(
    config: &'a Config,
    remaining: &BTreeSet<String>,
    name: &'a str,
    state: &mut BTreeMap<&'a str, u8>,
    stack: &mut Vec<&'a str>,
    out: &mut Vec<String>,
) -> Option<()> {
    match state.get(name).copied() {
        Some(1) => {
            let start = stack.iter().position(|entry| *entry == name).unwrap_or(0);
            out.extend(stack[start..].iter().map(|entry| (*entry).to_owned()));
            out.push(name.to_owned());
            return Some(());
        }
        Some(2) => return None,
        _ => {}
    }

    state.insert(name, 1);
    stack.push(name);

    for dependency in &config.services[name].needs {
        if !remaining.contains(dependency) {
            continue;
        }
        if visit(config, remaining, dependency, state, stack, out).is_some() {
            return Some(());
        }
    }

    stack.pop();
    state.insert(name, 2);
    None
}

fn cycle_from(config: &Config, remaining: &BTreeSet<String>, start: &str) -> Option<Vec<String>> {
    let mut state: BTreeMap<&str, u8> = BTreeMap::new();
    let mut stack: Vec<&str> = Vec::new();
    let mut cycle = Vec::new();
    visit(config, remaining, start, &mut state, &mut stack, &mut cycle);
    (!cycle.is_empty()).then_some(cycle)
}
