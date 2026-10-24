//! Deep merge for `burrow.local.toml`.

use toml::Value;

pub(crate) fn merge_values(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Table(base), Value::Table(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(existing) => merge_values(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::merge_values;
    use toml::Value;

    #[test]
    fn tables_merge_and_scalars_replace() {
        let mut base: Value = toml::from_str(
            r#"
            [project]
            name = "shop"
            [services.api]
            run = "npm run dev"
            memory = "256MB"
            "#,
        )
        .expect("valid base");
        let overlay: Value = toml::from_str(
            r#"
            [services.api]
            memory = "512MB"
            [services.api.env]
            DEBUG = "1"
            "#,
        )
        .expect("valid overlay");

        merge_values(&mut base, overlay);

        assert_eq!(base["services"]["api"]["run"].as_str(), Some("npm run dev"));
        assert_eq!(base["services"]["api"]["memory"].as_str(), Some("512MB"));
        assert_eq!(base["services"]["api"]["env"]["DEBUG"].as_str(), Some("1"));
    }
}
