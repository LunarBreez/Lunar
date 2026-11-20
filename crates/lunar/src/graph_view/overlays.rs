//! Éléments superposés au graphe : légende, témoin de la gravité, boutons
//! de zoom, et pièces du bandeau bas. Les textes et les couleurs viennent des
//! mêmes sources que le rendu (`lunar_ui_logic::graph::legend`,
//! `protocol_colors`).

use gpui::{
    AnyElement, App, ClickEvent, FontWeight, Hsla, PathBuilder, Pixels, Window, canvas, div, point,
    prelude::*, px, relative, rgb, rgba,
};
use lunar_ui_logic::graph::legend::{
    GRAPH_RENDERING_LEGEND, format_legend_number, node_color_legend,
};
use lunar_ui_logic::graph::protocol_colors::protocol_color_legend;
use lunar_ui_logic::graph::style::parse_hex;

use super::font::LABEL_FONT_FAMILY;
use super::paint::hsla;
use crate::assets::Icon;
use crate::widgets::button::{Button, ButtonSize};

/// Rappel de clic d'un élément superposé.
pub type Handler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// Couleur `#rrggbb` du modèle en couleur gpui.
fn hex(color: &str) -> Hsla {
    hsla(parse_hex(color))
}

/// Séparateur vertical du bandeau bas (`.sep`).
pub fn separator() -> impl IntoElement {
    div().w(px(1.)).h(px(20.)).bg(rgb(0x333333)).flex_none()
}

/// Témoin de la gravité, en haut à gauche (`.force-button`, `.on`) : le
/// bouton lui-même est dans la barre d'outils de l'application.
pub fn force_indicator(on: bool, label: &'static str, title: &'static str) -> impl IntoElement {
    div()
        .id("graph-force-indicator")
        .absolute()
        .top(px(10.))
        .left(px(10.))
        .px(px(14.))
        .py(px(8.))
        .rounded(px(8.))
        .bg(rgb(0x0b1b25))
        .text_color(rgb(0xffffff))
        .text_size(px(13.))
        .border_2()
        .border_color(if on {
            hex("#1de9b6")
        } else {
            gpui::transparent_black()
        })
        .tooltip(crate::widgets::tooltip::tooltip(title))
        .child(label)
}

/// Boutons de zoom, en bas à gauche de la zone du graphe (la légende
/// occupe la droite).
pub fn zoom_controls(zoom_in: Handler, zoom_out: Handler, reset: Handler) -> impl IntoElement {
    div()
        .id("graph-zoom-controls")
        .occlude()
        .absolute()
        .bottom(px(10.))
        .left(px(10.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .p(px(4.))
        .rounded(px(8.))
        .bg(rgba(0x1a1a1ae6))
        .border_1()
        .border_color(rgb(0x3a3a3a))
        .child(
            Button::icon_only("graph-zoom-in", Icon::ZoomIn, "Zoomer")
                .ghost()
                .size(ButtonSize::Small)
                .on_click(zoom_in),
        )
        .child(
            Button::icon_only("graph-zoom-out", Icon::ZoomOut, "Dézoomer")
                .ghost()
                .size(ButtonSize::Small)
                .on_click(zoom_out),
        )
        .child(
            Button::icon_only(
                "graph-zoom-reset",
                Icon::FocusCentered,
                "Recadrer le graphe",
            )
            .ghost()
            .size(ButtonSize::Small)
            .on_click(reset),
        )
}

/// Pastille ronde d'une catégorie de nœud (cercle r 7, trait 2, sur 20 px).
fn node_swatch(fill: Hsla, stroke: Hsla, stroke_width: f32) -> impl IntoElement {
    div()
        .w(px(24.))
        .h(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .size(px(14. + stroke_width))
                .rounded_full()
                .bg(fill)
                .border(px(stroke_width))
                .border_color(stroke),
        )
}

/// Petit dessin vectoriel d'une ligne de légende (flèche, tunnel).
fn swatch_canvas(
    width: f32,
    height: f32,
    draw: impl Fn(gpui::Point<Pixels>, &mut Window) + 'static,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| draw(bounds.origin, window),
    )
    .w(px(width))
    .h(px(height))
    .flex_none()
}

/// Trace un trait de `a` à `b` (coordonnées locales) depuis `o`.
fn stroke_line(
    window: &mut Window,
    o: gpui::Point<Pixels>,
    a: (f32, f32),
    b: (f32, f32),
    w: f32,
    color: Hsla,
) {
    let mut path = PathBuilder::stroke(px(w));
    path.move_to(point(o.x + px(a.0), o.y + px(a.1)));
    path.line_to(point(o.x + px(b.0), o.y + px(b.1)));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// Largeur du contenu de la légende dépliée (330 − 2 × 14 − 2 × 1).
const LEGEND_CONTENT_WIDTH: f32 = 300.0;
/// Largeur d'une pastille et de son écart au texte (24 + 7).
const SWATCH_SPAN: f32 = 31.0;
/// Largeur d'une colonne de la grille (moitié, moins l'écart de 10).
const GRID_ITEM_WIDTH: f32 = LEGEND_CONTENT_WIDTH / 2.0 - 10.0;

/// Ligne de légende : pastille et texte sur `width` pixels. La largeur du
/// texte est fixée : gpui 0.2.2 mesure sinon le retour à la ligne sur une
/// largeur provisoire et réserve des lignes en trop.
fn legend_item(
    swatch: impl IntoElement,
    text: impl Into<gpui::SharedString>,
    width: f32,
) -> gpui::Div {
    legend_row(swatch, text, width, SWATCH_SPAN)
}

/// Ligne de légende dont la pastille et son écart occupent `span` pixels.
fn legend_row(
    swatch: impl IntoElement,
    text: impl Into<gpui::SharedString>,
    width: f32,
    span: f32,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .w(px(width))
        .gap(px(7.))
        .text_color(rgb(0xeaeaea))
        .text_size(px(11.))
        .line_height(relative(1.3))
        .child(swatch)
        .child(div().w(px(width - span)).child(text.into()))
}

/// Ligne de légende sur toute la largeur.
fn detail(swatch: impl IntoElement, text: impl Into<gpui::SharedString>) -> gpui::Div {
    legend_item(swatch, text, LEGEND_CONTENT_WIDTH).mt(px(7.))
}

/// Largeur des badges de zoom (« ≥ 1,2× »).
const BADGE_WIDTH: f32 = 46.0;

/// Ligne de légende à badge de zoom.
fn badge_detail(badge: impl IntoElement, text: impl Into<gpui::SharedString>) -> gpui::Div {
    legend_row(badge, text, LEGEND_CONTENT_WIDTH, BADGE_WIDTH + 7.0).mt(px(7.))
}

/// Titre de section (`h4`).
fn section_title(text: &'static str) -> impl IntoElement {
    div()
        .mb(px(8.))
        .text_color(rgb(0xbbbbbb))
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

/// Grille de deux colonnes (`.legend-grid`).
fn grid(items: Vec<AnyElement>) -> impl IntoElement {
    div().flex().flex_wrap().gap_y(px(5.)).mb(px(9.)).children(
        items
            .into_iter()
            .map(|item| div().w(px(LEGEND_CONTENT_WIDTH / 2.0)).child(item)),
    )
}

/// Légende du graphe, en haut à droite.
pub fn legend(expanded: bool, toggle: Handler) -> impl IntoElement {
    let r = GRAPH_RENDERING_LEGEND;
    let n = |v: f32| format_legend_number(v);
    let heading = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .pb(px(8.))
        .border_b_1()
        .border_color(rgb(0x3a3a3a))
        .child(
            div()
                .flex_1()
                .text_color(rgb(0xffffff))
                .text_size(px(16.))
                .font_weight(FontWeight::BOLD)
                .child("Légende du graphe"),
        )
        .child(
            div()
                .id("graph-legend-toggle")
                .px(px(6.))
                .py(px(3.))
                .border_1()
                .border_color(rgb(0x555555))
                .rounded(px(4.))
                .bg(rgb(0x242424))
                .text_color(rgb(0xffffff))
                .text_size(px(10.))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(0x2e2e2e)))
                .on_click(toggle)
                .child(if expanded { "Réduire" } else { "Afficher" }),
        );

    let mut panel = div()
        .id("graph-legend")
        .occlude()
        .max_h_full()
        .overflow_y_scroll()
        .bg(rgb(0x1a1a1a))
        .border_1()
        .border_color(rgb(0x3a3a3a))
        .rounded(px(8.))
        .p(px(14.))
        .text_color(rgb(0xeaeaea))
        .font_family(LABEL_FONT_FAMILY)
        .shadow(vec![gpui::BoxShadow {
            color: gpui::hsla(0., 0., 0., 0.5),
            offset: point(px(0.), px(4.)),
            blur_radius: px(12.),
            spread_radius: px(0.),
        }])
        .child(heading);
    if expanded {
        panel = panel.w(px(330.)).child(legend_content(r, n));
    }
    // Conteneur sans interaction : il borne la hauteur (100 % − 20 px) sans
    // capter la souris hors du panneau.
    div()
        .absolute()
        .top(px(10.))
        .right(px(10.))
        .bottom(px(10.))
        .flex()
        .flex_col()
        .items_end()
        .child(panel)
}

/// Sections de la légende dépliée.
fn legend_content(
    r: lunar_ui_logic::graph::legend::GraphRenderingLegend,
    n: impl Fn(f32) -> String,
) -> impl IntoElement {
    let muted_fill = gpui::hsla(0., 0., 0.92, 0.25);
    let nodes = node_color_legend()
        .into_iter()
        .map(|item| {
            legend_item(
                node_swatch(hex(item.color), hex(&item.border_color), 2.),
                item.label,
                GRID_ITEM_WIDTH,
            )
            .into_any_element()
        })
        .collect();
    let protocols = protocol_color_legend()
        .into_iter()
        .map(|item| {
            legend_item(
                div()
                    .w(px(24.))
                    .h(px(7.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .w(px(24.))
                            .h(px(3.5))
                            .rounded(px(1.))
                            .bg(hex(item.color)),
                    ),
                item.label,
                GRID_ITEM_WIDTH,
            )
            .into_any_element()
        })
        .collect();
    let light = rgb(0xeaeaea).into();
    let size_swatch = div()
        .w(px(28.))
        .h(px(26.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .size(px(3.))
                .rounded_full()
                .bg(rgb(0xb0bec5))
                .border(px(0.5))
                .border_color(rgb(0x78909c)),
        )
        .child(
            div()
                .size(px(23.))
                .rounded_full()
                .bg(rgb(0xb0bec5))
                .border(px(0.5))
                .border_color(rgb(0x78909c)),
        );
    let edge_scale = swatch_canvas(24., 16., move |o, window| {
        let k = 24. / 40.;
        stroke_line(window, o, (0.6, 3.6), (23.4, 3.6), 1.2 * k, light);
        stroke_line(window, o, (0.6, 11.4), (23.4, 11.4), 7. * k, light);
    });
    let arrow = swatch_canvas(24., 16., move |o, window| {
        let k = 24. / 32.;
        stroke_line(window, o, (0.75, 8.), (18.75, 8.), 1.5, light);
        let mut tri = PathBuilder::fill();
        tri.move_to(point(o.x + px(24. * k), o.y + px(8. - 4. * k)));
        tri.line_to(point(o.x + px(31. * k), o.y + px(8.)));
        tri.line_to(point(o.x + px(24. * k), o.y + px(8. + 4. * k)));
        tri.close();
        if let Ok(path) = tri.build() {
            window.paint_path(path, light);
        }
    });
    let dim_edge = hex(r.tunnel_dim_edge_color);
    let dim_node = hex(r.tunnel_dim_node_color);
    let tunnel = swatch_canvas(24., 16., move |o, window| {
        let k = 24. / 34.;
        stroke_line(window, o, (0.7, 4.4), (23.3, 4.4), 3. * k, light);
        stroke_line(window, o, (0.7, 11.6), (23.3, 11.6), 1.5, dim_edge);
        window.paint_quad(gpui::quad(
            gpui::Bounds::new(
                point(o.x + px(4. * k - 3. * k), o.y + px(11.6 - 3. * k)),
                gpui::size(px(6. * k), px(6. * k)),
            ),
            px(3. * k),
            dim_node,
            px(0.),
            gpui::transparent_black(),
            gpui::BorderStyle::Solid,
        ));
    });
    let badge = |text: String| {
        div()
            .w(px(BADGE_WIDTH))
            .flex_none()
            .flex()
            .justify_center()
            .px(px(5.))
            .py(px(1.))
            .rounded(px(4.))
            .border_1()
            .border_color(rgb(0x555555))
            .bg(rgb(0x242424))
            .text_color(rgb(0xffffff))
            .text_size(px(10.))
            .child(text)
    };
    let border_swatch = |color: &str| node_swatch(muted_fill, hex(color), 3.);

    div()
        .pt(px(14.))
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(
            div()
                .child(section_title("Nœuds"))
                .child(grid(nodes))
                .child(detail(
                    size_swatch,
                    format!(
                        "Taille : volume cumulé des flux reliés, échelle logarithmique ({} à {}).",
                        n(r.node_size.min),
                        n(r.node_size.max)
                    ),
                ))
                .child(detail(
                    border_swatch(r.mac_conflict_border_color),
                    "Bordure rouge : plusieurs MAC unicast observées pour la même IP.",
                ))
                .child(detail(
                    border_swatch(r.duplicate_ip_border_color),
                    "Bordure ambre : même IP présente sur plusieurs VLAN.",
                ))
                .child(detail(
                    border_swatch(r.gateway_mac_border_color),
                    "Bordure violette : MAC partagée par de nombreuses IP (passerelle probable), l'IP est derrière cette MAC.",
                )),
        )
        .child(
            div()
                .child(section_title("Arêtes — couleur par protocole"))
                .child(grid(protocols))
                .child(detail(
                    edge_scale,
                    format!(
                        "Épaisseur : octets du flux, échelle logarithmique ({} à {}).",
                        n(r.edge_size.min),
                        n(r.edge_size.max)
                    ),
                ))
                .child(detail(arrow, "Flèche : sens du premier paquet observé."))
                .child(
                    div()
                        .mt(px(5.))
                        .ml(px(SWATCH_SPAN))
                        .w(px(LEGEND_CONTENT_WIDTH - SWATCH_SPAN))
                        .text_color(rgb(0xaaaaaa))
                        .text_size(px(10.))
                        .line_height(relative(1.3))
                        .child("Les flux retour sont agrégés sur la même arête, sans style distinct."),
                ),
        )
        .child(
            div()
                .child(section_title("Zoom et ports"))
                .child(badge_detail(
                    badge(format!("≥ {}×", n(r.edge_label_zoom))),
                    "Protocole affiché sur l’arête.",
                ))
                .child(badge_detail(
                    badge(format!("≥ {}×", n(r.port_label_zoom))),
                    "Ports service ; « … » signale les ports dynamiques.",
                )),
        )
        .child(div().child(section_title("Tunnels")).child(detail(
            tunnel,
            "Survol : famille liée conservée, reste estompé. Clic : épingler ou libérer.",
        )))
}
