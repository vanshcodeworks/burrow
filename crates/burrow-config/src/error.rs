//! Configuration errors.

use std::path::PathBuf;

/// An error produced while loading or validating `burrow.toml`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A configuration file could not be read.
    #[error("could not read {path}: {source}")]
    Io {
        /// File that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The TOML (or its typed representation) was invalid.
    #[error("{file}: {message}")]
    Parse {
        /// File name shown to the user.
        file: String,
        /// Parser message.
        message: String,
    },

    /// A semantically invalid value.
    #[error("invalid configuration at `{path}`: {message}")]
    Invalid {
        /// Dotted key path such as `services.api.needs[0]`.
        path: String,
        /// Explanation and, when possible, a suggested fix.
        message: String,
    },

    /// The dependency graph contains a cycle.
    #[error("dependency cycle detected: {path}")]
    Cycle {
        /// The cycle, formatted as `a -> b -> a`.
        path: String,
    },
}

impl ConfigError {
    /// Builds an [`ConfigError::Invalid`] with a dotted key path.
    pub fn invalid(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Invalid {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// Returns the closest candidate to `needle`, if it is close enough to suggest.
pub(crate) fn suggest<'a, I>(needle: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let distance = levenshtein(needle, candidate);
        if best.is_none_or(|(best_distance, _)| distance < best_distance) {
            best = Some((distance, candidate));
        }
    }

    let (distance, candidate) = best?;
    let limit = (needle.len() / 3).max(1);
    (distance <= limit).then(|| candidate.to_owned())
}

fn levenshtein(left: &str, right: &str) -> usize {
    let right_chars: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right_chars.len()).collect();
    let mut current = vec![0usize; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution = previous[right_index] + usize::from(left_char != *right_char);
            let insertion = current[right_index] + 1;
            let deletion = previous[right_index + 1] + 1;
            current[right_index + 1] = substitution.min(insertion).min(deletion);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::suggest;

    #[test]
    fn suggests_a_near_miss() {
        let names = vec!["db".to_owned(), "cache".to_owned(), "api".to_owned()];
        assert_eq!(suggest("dbb", &names), Some("db".to_owned()));
        assert_eq!(suggest("apii", &names), Some("api".to_owned()));
    }

    #[test]
    fn does_not_suggest_unrelated_names() {
        let names = vec!["database".to_owned()];
        assert_eq!(suggest("web", &names), None);
    }
}
