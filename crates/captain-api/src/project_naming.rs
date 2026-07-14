pub(crate) fn title_from_goal(goal: &str) -> String {
    let mut title = goal
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");
    if title.len() > 80 {
        title.truncate(80);
    }
    title
}

pub(crate) fn slugify_project_name(name: &str) -> String {
    let mut slug = String::with_capacity(name.len().min(64));
    let mut last_dash = false;
    for ch in name.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            slug.push(ch);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
        if slug.len() >= 64 {
            break;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "project".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_from_goal_uses_the_first_eight_words() {
        assert_eq!(
            title_from_goal("ship the launch page and verify the install path today"),
            "ship the launch page and verify the install"
        );
    }

    #[test]
    fn title_from_goal_truncates_long_words_defensively() {
        let title = title_from_goal(&"a".repeat(120));
        assert_eq!(title.len(), 80);
    }

    #[test]
    fn slugify_project_name_keeps_ascii_lowercase_digits_and_dashes() {
        assert_eq!(
            slugify_project_name("Ship Captain V2: Release!"),
            "ship-captain-v2-release"
        );
    }

    #[test]
    fn slugify_project_name_is_bounded_and_has_fallback() {
        assert_eq!(slugify_project_name("!!!"), "project");
        assert!(slugify_project_name(&"abc ".repeat(40)).len() <= 64);
    }
}
