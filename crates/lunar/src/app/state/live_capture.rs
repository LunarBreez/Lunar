//! Démarrage et arrêt de la capture live.
//!
//! Le démarrage ouvre un canal neuf du store ([`Bridge::store_channel`]) :
//! les événements de la session (`Started`,
//! `Stats`, `PacketBatch`, `GraphBatch`, `ChannelCapacityPayload`,
//! `Stopped`) passent par la pompe de [`AppState`] et le store de capture,
//! puis vont au graphe, au journal de trames et à la jauge du canal.
//! L'arrêt passe le canal courant du store ([`Bridge::current_store_channel`])
//! pour que `Stopped` et le dernier bilan y
//! arrivent. Les gardes et l'application des statuts sont dans
//! [`lunar_ui_logic::live_capture`].
//!
//! Traces attendues par le harnais E2E : « Capture démarrée : true » et
//! « Capture arrêtée : false ».
//!
//! [`Bridge::store_channel`]: crate::app::bridge::Bridge::store_channel
//! [`Bridge::current_store_channel`]: crate::app::bridge::Bridge::current_store_channel

use gpui::Context;

use super::AppState;

impl AppState {
    /// Démarre la capture live sur l'interface configurée.
    pub(super) fn start_capture(&mut self, cx: &mut Context<Self>) {
        let panel_open = self.panels.toolbar_panel_open();
        if self
            .live
            .begin_start(&mut self.capture, panel_open)
            .is_err()
        {
            return;
        }
        self.changed(cx);
        let channel = self.bridge.store_channel();
        self.bridge
            .run(
                cx,
                move |lunar| lunar.start_capture(channel),
                |this, result, cx| {
                    if let Err(err) = this.live.finish_start(&mut this.capture, result) {
                        this.show_error(&err, cx);
                    }
                    this.changed(cx);
                },
            )
            .detach();
    }

    /// Arrête la capture live ; la présence de données est relue ensuite.
    pub(super) fn stop_capture(&mut self, cx: &mut Context<Self>) {
        if self.live.begin_stop(&self.capture).is_err() {
            return;
        }
        let channel = self.bridge.current_store_channel();
        self.bridge
            .run(
                cx,
                move |lunar| lunar.stop_capture(channel),
                |this, result, cx| {
                    if let Err(err) = this.live.finish_stop(&mut this.capture, result) {
                        this.show_error(&err, cx);
                    }
                    this.refresh_flags(cx);
                    this.changed(cx);
                },
            )
            .detach();
    }
}
