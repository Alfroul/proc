use ratatui::text::Span;
use ratatui::style::{Color, Style};

use crate::app::App;
use crate::security::SecurityScore;

pub fn draw_score(app: &App, pid: u32) -> Span<'static> {
    let score = match app.security_scores.get(&pid) {
        Some(s) => s,
        None => return Span::raw(""),
    };

    score_span(score)
}

pub fn score_span(score: &SecurityScore) -> Span<'static> {
    if score.score >= 90 {
        // Default safe, reduce noise
        Span::raw("")
    } else if score.score >= 60 {
        Span::styled(
            format!("{}", score.score),
            Style::default().fg(Color::Yellow),
        )
    } else if score.score >= 30 {
        Span::styled(
            format!("!{}", score.score),
            Style::default().fg(Color::Rgb(255, 165, 0)),
        )
    } else {
        Span::styled(
            format!("!!{}", score.score),
            Style::default().fg(Color::Red),
        )
    }
}

pub fn score_style(score: u32) -> Style {
    if score >= 90 {
        Style::default()
    } else if score >= 60 {
        Style::default().fg(Color::Yellow)
    } else if score >= 30 {
        Style::default().fg(Color::Rgb(255, 165, 0))
    } else {
        Style::default().fg(Color::Red)
    }
}
