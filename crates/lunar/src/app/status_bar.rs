//! Barre de statut : interface, durée, CPU, compteurs et jauge du canal.
//!
//! À gauche : interface, type de liaison, filtre actif (effaçable), bilan
//! du dernier import, motif du dernier arrêt. À droite : durée, CPU, puis
//! les compteurs (reçues, flux, perdues noyau / interface / application,
//! rejets du parseur) et la jauge du canal.

use std::time::{Duration, Instant};

use gpui::{
    App, ClickEvent, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use lunar_ui_logic::capture_store::{CaptureStore, channel_occupancy_percent};

use crate::assets::{Icon, icon};
use crate::theme::Theme;
use crate::widgets::ellipsis;
use crate::widgets::tooltip::tooltip;

/// Hauteur de la barre (`.status-bar { height: 22px }`).
pub const STATUS_BAR_HEIGHT: f32 = 22.;

/// Couleur des badges DLT et import (`#9fd3a8`).
const BADGE_GREEN: u32 = 0x9f_d3_a8;

/// Couleur du badge d'arrêt (`#e0a458`).
const BADGE_STOP: u32 = 0xe0_a4_58;

/// Couleur du texte du filtre (`#8ab4ff`).
const FILTER_TEXT: u32 = 0x8a_b4_ff;

/// Durée de capture : repart de zéro à chaque démarrage, figée à l'arrêt.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureTimer {
    /// Démarrage de la capture en cours.
    started: Option<Instant>,
    /// Durée figée au dernier arrêt.
    frozen: Duration,
}

impl CaptureTimer {
    /// Suit l'état de la capture : un démarrage remet à zéro, un arrêt fige.
    pub fn sync(&mut self, running: bool, now: Instant) {
        match (running, self.started) {
            (true, None) => {
                self.started = Some(now);
                self.frozen = Duration::ZERO;
            }
            (false, Some(started)) => {
                self.frozen = now.saturating_duration_since(started);
                self.started = None;
            }
            _ => {}
        }
    }

    /// Durée écoulée à `now`.
    pub fn elapsed(&self, now: Instant) -> Duration {
        match self.started {
            Some(started) => now.saturating_duration_since(started),
            None => self.frozen,
        }
    }
}

/// Durée au format `m:ss`.
pub fn format_elapsed(elapsed: Duration) -> String {
    let total = elapsed.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}

/// Usage CPU à deux décimales (`cpuUsage.toFixed(2)`).
pub fn format_cpu(usage: f32) -> String {
    format!("{usage:.2}%")
}

/// Occupation du canal capture → traitement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChannelGauge {
    /// Occupation, en pourcentage arrondi.
    pub percent: u32,
    /// Canal saturé : le pipeline applique la backpressure.
    pub backpressure: bool,
}

impl ChannelGauge {
    /// Jauge depuis un `ChannelCapacityPayload`.
    pub fn from_payload(channel_size: usize, current_size: usize, backpressure: bool) -> Self {
        Self {
            percent: channel_occupancy_percent(channel_size, current_size),
            backpressure,
        }
    }
}

/// Ce que la barre affiche.
pub struct StatusBarData<'a> {
    /// État de la capture.
    pub capture: &'a CaptureStore,
    /// Durée de capture.
    pub elapsed: Duration,
    /// Usage CPU.
    pub cpu_usage: f32,
    /// Jauge du canal.
    pub channel: ChannelGauge,
}

/// Compteur `valeur :icône` avec bulle.
fn counter(
    id: &'static str,
    theme: &Theme,
    value: u64,
    kind: Icon,
    title: SharedString,
) -> impl IntoElement + use<> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .tooltip(tooltip(title))
        .child(
            div()
                .min_w(px(36.))
                .text_right()
                .font_family(theme.font_mono.clone())
                .child(format!("{value} :")),
        )
        .child(icon(kind).size(px(14.)).text_color(theme.text))
}

/// Badge monospace coloré de la partie gauche.
fn badge(
    id: &'static str,
    theme: &Theme,
    color: Hsla,
    text: String,
    title: String,
) -> gpui::Stateful<gpui::Div> {
    ellipsis(
        div()
            .id(id)
            .ml(px(10.))
            .max_w(px(260.))
            .font_family(theme.font_mono.clone())
            .text_size(theme.text_xs)
            .text_color(color)
            .tooltip(tooltip(title))
            .child(text),
    )
}

/// Barre de statut. `on_clear_filter` : bouton ✕ du filtre actif.
pub fn render_status_bar(
    data: StatusBarData<'_>,
    theme: &Theme,
    on_clear_filter: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let capture = data.capture;
    let stats = capture.live_stats;

    let mut left = div()
        .flex()
        .items_center()
        .min_w_0()
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .child(format!("Interface : {}", capture.interface)),
        );
    if !capture.link_type.is_empty() {
        left = left.child(badge(
            "statut-dlt",
            theme,
            rgb(BADGE_GREEN).into(),
            format!("DLT : {}", capture.link_type),
            "Type de liaison (LINKTYPE/DLT) de la capture ou du fichier importé".to_owned(),
        ));
    }
    if !capture.active_filter.is_empty() {
        let filter = capture.active_filter.clone();
        left = left.child(
            div()
                .id("statut-filtre")
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .ml(px(10.))
                .px(px(7.))
                .h(px(16.))
                .max_w(px(320.))
                .overflow_hidden()
                .rounded(theme.radius_sm)
                .border_1()
                .border_color(gpui::rgba(0x4a7c_ff66))
                .bg(gpui::rgba(0x4a7c_ff26))
                .tooltip(tooltip(filter.clone()))
                .child(icon(Icon::Flask).size(px(11.)).text_color(theme.text))
                .child(ellipsis(
                    div()
                        .min_w_0()
                        .font_family(theme.font_mono.clone())
                        .text_size(theme.text_xs)
                        .text_color(rgb(FILTER_TEXT))
                        .child(filter),
                ))
                .child(
                    div()
                        .id("statut-filtre-effacer")
                        .flex_none()
                        .cursor_pointer()
                        .tooltip(tooltip("Supprimer le filtre"))
                        .hover(|s| s.text_color(gpui::white()))
                        .on_click(on_clear_filter)
                        .child(icon(Icon::Close).size(px(11.)).text_color(theme.text_muted)),
                ),
        );
    }
    if let Some(last) = &capture.last_import {
        left = left.child(badge(
            "statut-import",
            theme,
            rgb(BADGE_GREEN).into(),
            last.badge_text(),
            last.report_detail(),
        ));
    }
    if let Some(reason) = &capture.last_stop_reason {
        left = left.child(badge(
            "statut-arret",
            theme,
            rgb(BADGE_STOP).into(),
            format!("Arrêt : {reason}"),
            reason.clone(),
        ));
    }

    let gauge_color: Hsla = if data.channel.backpressure {
        rgb(0xff_52_52).into()
    } else {
        rgb(0xe7_9a_6e).into()
    };
    let channel_title = if data.channel.backpressure {
        "Canal saturé : le pipeline applique la backpressure".to_owned()
    } else {
        format!("{}%", data.channel.percent)
    };

    let right = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .id("statut-duree")
                .flex()
                .items_center()
                .gap(px(4.))
                .tooltip(tooltip("Durée d'exécution"))
                .child(icon(Icon::Clock).size(px(14.)).text_color(theme.text))
                .child(format!(": {}", format_elapsed(data.elapsed))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(icon(Icon::Cpu).size(px(16.)).text_color(theme.text))
                .child(format_cpu(data.cpu_usage)),
        )
        .child(counter(
            "statut-recues",
            theme,
            stats.received,
            Icon::Inbox,
            "Trames reçues 📥 par la carte réseau".into(),
        ))
        .child(counter(
            "statut-flux",
            theme,
            stats.processed,
            Icon::ChartBar,
            "Flux distincts dans la matrice 📊".into(),
        ))
        .child(counter(
            "statut-noyau",
            theme,
            stats.dropped,
            Icon::CircleX,
            "Trames ❌ perdues côté kernel".into(),
        ))
        .child(counter(
            "statut-interface",
            theme,
            stats.if_dropped,
            Icon::Ban,
            "Trames 🚫 perdues au niveau de l’interface".into(),
        ))
        .child(counter(
            "statut-application",
            theme,
            stats.app_dropped,
            Icon::Warning,
            "Trames ⚠️ perdues côté application (pool ou canal saturé)".into(),
        ))
        .child(counter(
            "statut-rejets",
            theme,
            stats.rejected_total(),
            Icon::Puzzle,
            format!(
                "Trames 🧩 acceptées mais illisibles par le parseur\n{}",
                stats.rejected_detail()
            )
            .into(),
        ))
        .child(
            div()
                .id("statut-canal")
                .flex()
                .items_center()
                .gap(px(4.))
                .tooltip(tooltip(channel_title))
                .child(icon(Icon::Alarm).size(px(14.)).text_color(theme.text))
                .child(format!(": {}%", data.channel.percent))
                .child(
                    div()
                        .w(px(100.))
                        .h(px(3.))
                        .rounded(px(2.))
                        .bg(rgb(0xf7_f7_f7))
                        .overflow_hidden()
                        .child(
                            div()
                                .h_full()
                                .w(px(data.channel.percent.min(100) as f32))
                                .bg(gauge_color),
                        ),
                ),
        );

    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap(px(10.))
        .h(px(STATUS_BAR_HEIGHT))
        .w_full()
        .px(px(10.))
        .bg(theme.bg_status_bar)
        .text_color(gpui::white())
        .text_size(theme.text_sm)
        .child(left)
        .child(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duree_repart_de_zero_et_se_fige() {
        let t0 = Instant::now();
        let mut timer = CaptureTimer::default();
        assert_eq!(timer.elapsed(t0), Duration::ZERO);
        timer.sync(true, t0);
        assert_eq!(
            timer.elapsed(t0 + Duration::from_secs(75)),
            Duration::from_secs(75)
        );
        timer.sync(false, t0 + Duration::from_secs(90));
        assert_eq!(
            timer.elapsed(t0 + Duration::from_secs(500)),
            Duration::from_secs(90)
        );
        // Nouveau démarrage : repart de zéro.
        let t1 = t0 + Duration::from_secs(600);
        timer.sync(true, t1);
        assert_eq!(
            timer.elapsed(t1 + Duration::from_secs(3)),
            Duration::from_secs(3)
        );
        // Un second « en cours » ne remet pas à zéro.
        timer.sync(true, t1 + Duration::from_secs(5));
        assert_eq!(
            timer.elapsed(t1 + Duration::from_secs(6)),
            Duration::from_secs(6)
        );
    }

    #[test]
    fn formats() {
        assert_eq!(format_elapsed(Duration::from_secs(0)), "0:00");
        assert_eq!(format_elapsed(Duration::from_secs(75)), "1:15");
        assert_eq!(format_elapsed(Duration::from_millis(3_599_999)), "59:59");
        assert_eq!(format_elapsed(Duration::from_secs(3_600)), "60:00");
        assert_eq!(format_cpu(12.3456), "12.35%");
        assert_eq!(format_cpu(0.0), "0.00%");
    }

    #[test]
    fn jauge_du_canal() {
        assert_eq!(
            ChannelGauge::from_payload(1024, 256, false),
            ChannelGauge {
                percent: 25,
                backpressure: false
            }
        );
        assert_eq!(ChannelGauge::from_payload(0, 10, true).percent, 0);
    }
}
