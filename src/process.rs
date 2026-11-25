//! Détection d'un travail en cours dans un terminal (logique pure).
//!
//! Le shell d'un onglet est chef de sa session : tant qu'il attend une
//! commande, le groupe de processus au premier plan du terminal est le sien.
//! Si ce n'est plus le cas, une commande tourne (`sleep`, `vim`, `make`…).

use std::time::Duration;

/// Analyse le contenu de `/proc/<pid>/stat` du shell : vrai si le groupe de
/// processus au premier plan du terminal (`tpgid`) n'est pas celui du shell.
///
/// Le nom de commande (2e champ) peut contenir des espaces et des
/// parenthèses : on repart après la *dernière* `)`.
pub fn foreground_job_running(stat: &str) -> bool {
    let Some((_, after_name)) = stat.rsplit_once(')') else {
        return false;
    };
    // Après le nom : état, ppid, pgrp, session, tty_nr, tpgid, …
    let mut fields = after_name.split_whitespace().skip(2);
    let (Some(pgrp), Some(_session), Some(_tty), Some(tpgid)) = (
        fields.next().and_then(|field| field.parse::<i64>().ok()),
        fields.next(),
        fields.next(),
        fields.next().and_then(|field| field.parse::<i64>().ok()),
    ) else {
        return false;
    };
    // `tpgid` vaut -1 sans terminal de contrôle : rien à signaler.
    tpgid > 0 && tpgid != pgrp
}

/// Durée minimale d'une commande pour qu'on notifie sa fin.
pub const NOTIFY_MIN_DURATION: Duration = Duration::from_secs(10);

/// Suit l'alternance « commande en cours / shell au repos » d'un terminal,
/// observée à intervalle régulier.
#[derive(Debug, Default)]
pub struct JobTracker {
    started: Option<Duration>,
}

impl JobTracker {
    /// `busy` : une commande tourne ; `now` : temps écoulé depuis un point fixe.
    /// Retourne la durée de la commande au moment où elle se termine.
    pub fn observe(&mut self, busy: bool, now: Duration) -> Option<Duration> {
        match (busy, self.started) {
            (true, None) => {
                self.started = Some(now);
                None
            }
            (false, Some(start)) => {
                self.started = None;
                Some(now.saturating_sub(start))
            }
            _ => None,
        }
    }
}

/// Notifier seulement les commandes longues, et seulement si l'utilisateur ne
/// regarde pas déjà l'onglet (fenêtre inactive ou autre onglet affiché).
pub fn should_notify(
    duration: Duration,
    minimum: Duration,
    window_active: bool,
    tab_selected: bool,
) -> bool {
    duration >= minimum && !(window_active && tab_selected)
}

/// Durée lisible : « 42 s », « 3 min 05 s », « 1 h 02 min ».
pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match seconds {
        0..=59 => format!("{seconds} s"),
        60..=3599 => format!("{} min {:02} s", seconds / 60, seconds % 60),
        _ => format!("{} h {:02} min", seconds / 3600, seconds % 3600 / 60),
    }
}

/// Lit l'état du processus `pid` ; un processus disparu n'a pas de travail.
pub fn is_busy(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| foreground_job_running(&stat))
}

#[cfg(test)]
mod tests {
    use super::*;

    // pid (comm) état ppid pgrp session tty_nr tpgid …
    #[test]
    fn idle_shell_owns_the_foreground() {
        assert!(!foreground_job_running(
            "4242 (zsh) S 4000 4242 4242 34816 4242 4194304 1 0"
        ));
    }

    #[test]
    fn a_running_command_takes_the_foreground() {
        assert!(foreground_job_running(
            "4242 (zsh) S 4000 4242 4242 34816 5151 4194304 1 0"
        ));
    }

    #[test]
    fn command_names_with_spaces_and_parentheses_are_handled() {
        assert!(!foreground_job_running(
            "4242 (my (odd) sh) S 4000 4242 4242 34816 4242 0"
        ));
        assert!(foreground_job_running(
            "4242 (my (odd) sh) S 4000 4242 4242 34816 9000 0"
        ));
    }

    #[test]
    fn no_controlling_terminal_or_garbage_is_not_busy() {
        assert!(!foreground_job_running(
            "4242 (zsh) S 4000 4242 4242 0 -1 4194304"
        ));
        assert!(!foreground_job_running(""));
        assert!(!foreground_job_running("pas un stat"));
        assert!(!foreground_job_running("1 (x) S 1 2"));
    }

    fn secs(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    #[test]
    fn tracker_reports_the_duration_when_a_command_ends() {
        let mut tracker = JobTracker::default();
        assert_eq!(tracker.observe(false, secs(1)), None);
        assert_eq!(tracker.observe(true, secs(2)), None);
        assert_eq!(tracker.observe(true, secs(30)), None);
        assert_eq!(tracker.observe(false, secs(42)), Some(secs(40)));
        assert_eq!(tracker.observe(false, secs(43)), None);
        // Une seconde commande est suivie séparément.
        assert_eq!(tracker.observe(true, secs(50)), None);
        assert_eq!(tracker.observe(false, secs(51)), Some(secs(1)));
    }

    #[test]
    fn notification_needs_a_long_command_and_an_unwatched_tab() {
        let long = secs(60);
        let min = NOTIFY_MIN_DURATION;
        assert!(should_notify(long, min, false, true));
        assert!(should_notify(long, min, true, false));
        assert!(should_notify(long, min, false, false));
        assert!(!should_notify(long, min, true, true));
        assert!(!should_notify(secs(3), min, false, false));
        assert!(should_notify(min, min, false, false));
    }

    #[test]
    fn durations_are_readable() {
        assert_eq!(format_duration(secs(0)), "0 s");
        assert_eq!(format_duration(secs(42)), "42 s");
        assert_eq!(format_duration(secs(185)), "3 min 05 s");
        assert_eq!(format_duration(secs(3725)), "1 h 02 min");
    }

    #[test]
    fn vanished_process_is_not_busy() {
        assert!(!is_busy(i32::MAX));
    }
}
