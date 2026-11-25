//! OS notifications (`lib/notifications.ts`). In-app toasts live on `AletheApp`; this only reaches
//! the system when the window is not in front.

/// Shows a desktop notification without blocking the UI thread.
pub fn os_notify(title: String, body: String) {
    std::thread::spawn(move || {
        let _ = notify_rust::Notification::new()
            .appname("Alethe")
            .summary(&title)
            .body(&body)
            .show();
    });
}

/// The last two segments of a path, like `shortPath` in `agentCompletionMonitor.ts`.
pub fn short_path(path: &str) -> String {
    let cleaned = path.trim_end_matches(['/', '\\']);
    let parts: Vec<&str> = cleaned
        .split(['/', '\\'])
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() <= 2 {
        return cleaned.to_string();
    }
    format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
}

#[cfg(test)]
mod tests {
    #[test]
    fn short_path_keeps_last_two_segments() {
        assert_eq!(super::short_path(r"C:\Users\me\code\app\"), "code/app");
        assert_eq!(super::short_path("/home/app"), "/home/app");
    }
}
