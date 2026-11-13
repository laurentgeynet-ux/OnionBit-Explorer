// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Display list (étape 22) : l'arbre de layout devient une
//! **liste ordonnée de primitives de dessin** — la seule
//! chose que la rasterisation (étape 23) consomme.
//!
//! Ordre de peinture — simplification bornée de CSS 2.1
//! Appendix E :
//! 1. fond (`background-color`, border-box) puis bordures
//!    de la boîte ;
//! 2. texte des lignes en-ligne (fragments non atomiques) ;
//! 3. enfants **non positionnés** dans l'ordre de l'arbre
//!    (les flottants y coulent à leur place — approximation
//!    documentée) ;
//! 4. enfants **positionnés** (`relative`/`absolute`/`fixed`)
//!    triés stablement par `z-index` croissant — un contexte
//!    d'empilement complet (transforms, opacity) est reporté.
//!
//! `overflow` ≠ `visible` → `PushClip`/`PopClip` autour des
//! descendants (boîte de padding, approximation sans
//! arrondis). `display: none` ne produit déjà pas de boîte.

use explorer_css::value::Color;
use explorer_css::value::{parse_color, parse_length, Unit};
use explorer_layout::font::ShapedRun;
use explorer_layout::text::{is_css_ws, trim_css_ws};
use explorer_layout::{
    scrollable_overflow_bottom, BoxKind, Float, LayoutBox, Overflow, Position, Rect,
};

/// Une primitive de dessin, dans l'ordre de peinture.
/// Opération `filter` CSS (R-M5/R-M11) — `url()` SVG
/// volontairement hors scope.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterOp {
    /// `blur(<longueur>)` — rayon px.
    Blur(f32),
    /// `brightness(<f|%>)` — facteur.
    Brightness(f32),
    /// `contrast(<f|%>)` — facteur.
    Contrast(f32),
    /// `grayscale(<f|%>)` — part 0..1.
    Grayscale(f32),
    /// `invert(<f|%>)` — part 0..1.
    Invert(f32),
    /// `opacity(<f|%>)` — facteur alpha.
    Opacity(f32),
    /// `saturate(<f|%>)` — facteur.
    Saturate(f32),
    /// `sepia(<f|%>)` — part 0..1.
    Sepia(f32),
    /// `hue-rotate(<angle>)` — degrés.
    HueRotate(f32),
    /// `drop-shadow(<dx> <dy> [<blur>] [<color>])` (R-M11)
    /// — ombre du **contenu** (forme alpha), pas d'une
    /// boîte : au raster, le canal alpha de l'offscreen est
    /// extrait, décalé, teinté `color`, flouté et composité
    /// SOUS le groupe filtré.
    DropShadow {
        /// Décalage horizontal px.
        dx: f32,
        /// Décalage vertical px.
        dy: f32,
        /// Rayon de flou px (0 = net).
        blur: f32,
        /// Couleur de l'ombre.
        color: Color,
    },
}

#[derive(Debug, Clone)]
pub enum DisplayItem {
    /// Rectangle plein (fond, côté de bordure, placeholder).
    FillRect {
        /// Zone couverte (border-box pour un fond).
        rect: Rect,
        /// Couleur RGBA.
        color: Color,
        /// Rayons des 4 coins \[tl,tr,br,bl] px (R-167) —
        /// 0 = rectangle franc.
        radii: [f32; 4],
        /// Formes des 4 coins \[tl,tr,br,bl] : exposant `K`
        /// de la superellipse (css-borders-4 §3.7 — R-270 ;
        /// `round`=1, `squircle`=2, `square`=+∞, `bevel`=0,
        /// `scoop`=-1, `notch`=-∞). 1 = coin rond classique.
        corners: [f32; 4],
    },
    /// Trait de `text-decoration` stylé (R-178) — `style` ∈
    /// `explorer_layout::inline::DECO_*` ; `double`/`dotted`/
    /// `dashed`/`wavy` se dessinent en patterns au raster,
    /// pas en plein.
    DecoRect {
        /// Slot du trait (pleine épaisseur, marge comprise).
        rect: Rect,
        /// Couleur RGBA.
        color: Color,
        /// `DECO_*` — voir `explorer_layout::inline`.
        style: u8,
    },
    /// Ombre portée d'une boîte (`box-shadow`, R-M2) —
    /// peinte SOUS le fond/bordure de son élément (App. E).
    BoxShadow {
        /// Border box de l'élément (avant offset/spread).
        rect: Rect,
        /// Décalage `(dx, dy)` de l'ombre.
        offset: (f32, f32),
        /// Rayon de flou px (approx. en bandes alpha au
        /// raster — pas de blur gaussien).
        blur: f32,
        /// Expansion px de la boîte d'ombre.
        spread: f32,
        /// Couleur de l'ombre.
        color: Color,
        /// Rayons des coins `[tl,tr,br,bl]` (suivent
        /// `border-radius`, R-167).
        radii: [f32; 4],
    },
    /// Bordure d'une boîte — 4 côtés uniformisés en un item.
    Border {
        /// Border box extérieure.
        rect: Rect,
        /// Épaisseurs `[haut, droite, bas, gauche]`.
        widths: [f32; 4],
        /// Couleurs par côté (même ordre).
        colors: [Color; 4],
        /// Style (`solid`/`dashed`/`dotted`/…) par côté.
        styles: [BorderStyle; 4],
        /// Rayons des 4 coins \[tl,tr,br,bl] px (R-167).
        radii: [f32; 4],
        /// Formes des 4 coins \[tl,tr,br,bl] : exposant `K`
        /// superellipse (R-270, même convention que
        /// `FillRect::corners` — 1 = rond).
        corners: [f32; 4],
    },
    /// Run de glyphes shapés, posé depuis l'origine
    /// baseline (`x`, `y` = ligne de base).
    Text {
        /// Origine : x du fragment, y de la baseline.
        origin: (f32, f32),
        /// Glyphes + métriques produits par le shaping —
        /// `Arc` : le clone d'item est O(1) (R-37, re-usage
        /// de plages dans le build incrémental).
        run: std::sync::Arc<ShapedRun>,
        /// Requête fonte — le raster résout les octets via
        /// `FontSource` pour les contours de glyphes.
        font: explorer_layout::font::FontRequest,
        /// `color` hérité du conteneur.
        color: Color,
        /// Taille utilisée (px) — pour le raster de glyphes.
        size_px: f32,
    },
    /// Contenu remplacé non encore dessiné (`img`, `iframe`) —
    /// cadre + étiquette pour le wireframe.
    Placeholder {
        /// Zone réservée.
        rect: Rect,
        /// Étiquette (`alt`, tag…).
        label: String,
    },
    /// Bitmap décodée, blittée dans `rect` (mise à l'échelle
    /// nearest — aucun lissage à cette étape).
    Image {
        /// Zone de destination (border box de l'élément).
        rect: Rect,
        /// Pixels décodés.
        img: std::sync::Arc<crate::image::DecodedImage>,
    },
    /// `text-emphasis` (R-186) : un point/signet d'emphase
    /// par glyphe du run, au-dessus (ou dessous selon
    /// `text-emphasis-position`) — marque peinte par le
    /// raster aux positions de plume de chaque glyphe.
    Emphasis {
        /// Origine du run (coords document).
        origin: (f32, f32),
        /// Run shapé (positions de plume des glyphes).
        run: std::sync::Arc<ShapedRun>,
        /// Taille de fonte (la marque fait ~0.5em).
        size_px: f32,
        /// Marque : 0 dot, 1 circle, 2 double-circle,
        /// 3 triangle, 4 sesame ; +8 si `open`.
        kind: u8,
        /// Couleur de la marque (`text-emphasis-color`
        /// ou `currentcolor`).
        color: Color,
        /// Position `over` (défaut horizontal) vs `under`.
        over: bool,
    },
    /// `background-image` décodée, tuilée dans `rect` selon
    /// `background-repeat` — origine padding box, aire border
    /// box. La tuile est mise à l'échelle selon
    /// `background-size` à l'émission (R-95).
    BackgroundImage {
        /// Aire de peinture (border box de la boîte porteuse) —
        /// les tuiles n'apparaissent que dedans.
        rect: Rect,
        /// Pixels décodés.
        img: std::sync::Arc<crate::image::DecodedImage>,
        /// Mode `background-repeat` `(x, y)` — 0 `repeat`,
        /// 1 `no-repeat`, 2 `space`, 3 `round`.
        repeat: (u8, u8),
        /// Coin haut-gauche de la tuile (0,0) en coords document
        /// — inclut `background-position` résolu ; pour
        /// `background-attachment: fixed`, la zone de
        /// positionnement est le viewport (R-13d).
        origin: (f32, f32),
        /// `background-blend-mode` de la couche (R-184) —
        /// 0 = `normal`, sinon tag `BLEND_MODES` ; appliqué
        /// comme `paint.blend_mode` à la tuile.
        blend: u8,
    },
    /// `background-color` passé par l'alpha d'un `mask-image`
    /// (R-52) : la couleur n'apparaît que là où le mask est
    /// opaque — motif « icône colorée » des design systems
    /// (`mask-image:url(icon.svg)` + `background-color` ; sans
    /// le mask la couleur remplirait la boîte entière → carré
    /// plein). `mask-size`/`mask-position`/`mask-repeat` sont
    /// résolus à l'émission dans `dst`/`repeat` (déviation
    /// documentée : `mask-size:auto` = étirement sur la zone,
    /// le motif dimensionnant la boîte à l'icône).
    MaskedFill {
        /// Boîte de destination (border box) — aire de
        /// peinture (mask-clip par défaut).
        rect: Rect,
        /// `background-color` résolue.
        color: Color,
        /// Bitmap du mask — seul l'alpha est lu.
        mask: std::sync::Arc<crate::image::DecodedImage>,
        /// Zone d'**une tuile** du mask (coords document) :
        /// `mask-size` + `mask-position` résolus — le bitmap
        /// y est mis à l'échelle en nearest puis tuilé selon
        /// `repeat`.
        dst: Rect,
        /// Mode `mask-repeat` `(x, y)` — 0 `repeat`,
        /// 1 `no-repeat`, 2 `space`, 3 `round`.
        repeat: (u8, u8),
        /// `mask-mode:luminance` — couverture = luma×alpha.
        luminance: bool,
    },
    /// `border-image-*` résolu (R-248) — image découpée en 9
    /// régions : coins copiés, bords étirés/tilés, centre
    /// étiré ou omis selon `fill`.
    BorderImage {
        /// Boîte de destination (border box + `outset`).
        rect: Rect,
        /// Bitmap source.
        img: std::sync::Arc<crate::image::DecodedImage>,
        /// Tranches de découpe `(top, right, bottom, left)`
        /// en pixels source (`border-image-slice` résolu).
        src_edges: [f32; 4],
        /// Épaisseurs de bordure `(top, right, bottom, left)`
        /// en pixels document (`border-image-width` résolu —
        /// `auto` → border-*-width calculé).
        dst_edges: [f32; 4],
        /// `fill` : la tranche centrale est peinte aussi.
        fill_center: bool,
        /// `border-image-repeat` `(x, y)` — 0 `stretch`,
        /// 1 `repeat` (tilé, approximation stretch si les
        /// deux bords ne tombent pas entiers), 2 `round`
        /// (tilé + ajustement à un nombre entier de tuiles),
        /// 3 `space` (tilé + espacement — approx `round`).
        repeat: (u8, u8),
    },
    /// Entrée de zone de clipping (`overflow` ≠ `visible`).
    PushClip(Rect),
    /// Entrée de zone de clipping arrondie (R-M9) :
    /// `overflow` ≠ `visible` + `border-radius` > 0 — la
    /// géométrie suit `rounded_rect_path` au raster.
    PushClipRounded {
        /// Zone de clipping (border box de la boîte).
        rect: Rect,
        /// Rayons des 4 coins `[tl,tr,br,bl]` px.
        radii: [f32; 4],
    },
    /// Sortie de la zone de clipping correspondante.
    PopClip,
    /// `transform: …` — matrice affine (origine déjà résolue
    /// dedans) appliquée aux items jusqu'au `PopTransform`
    /// correspondant ; les matrices s'empilent (imbrication).
    PushTransform(tiny_skia::Transform),
    /// Fin de portée du dernier `PushTransform`.
    PopTransform,
    /// Début d'un groupe filtré (`filter`, R-M5) : les
    /// items jusqu'à `PopFilter` se peignent hors-écran
    /// puis reçoivent les ops avant composite.
    PushFilter(Vec<FilterOp>),
    /// Fin du groupe filtré — composite du tampon avec
    /// les ops appliquées.
    PopFilter,
    /// `mix-blend-mode`/`isolation` (R-185) : les items
    /// jusqu'à `PopBlend` se peignent hors-écran puis se
    /// composite sur le fond accumulé avec le mode de fondu
    /// taggé (mêmes tags que `BLEND_MODES` ; `isolation`
    /// → tag 0, buffer isolant — les fondus des enfants ne
    /// traversent pas la borne).
    PushBlend(u8),
    /// Fin du groupe fondu.
    PopBlend,
    /// `mask-image` de groupe (R-252, css-masking-1) : les
    /// items jusqu'à `PopMask` se peignent hors-écran puis
    /// se compositent modulés par l'**alpha** du mask —
    /// l'élément masqué cache TOUT son rendu (fond,
    /// bordures, texte, descendants), pas seulement le
    /// `background-color` du motif `MaskedFill`. Posé
    /// **outermost** des brackets de la boîte (spec :
    /// filter → clip → mask). Résidus documentés :
    /// `mask-mode:luminance`, `mask-composite`, `mask-clip`/
    /// `mask-origin` non initiaux, multi-couches — la
    /// couche 0 seule est appliquée.
    PushMask {
        /// Aire de peinture du mask (border box, coords
        /// document — `mask-clip` résolu).
        rect: Rect,
        /// Couches `mask-image` résolues (R-252 suite —
        /// multi-couches) : chaque entrée = `(bitmap, tuile,
        /// repeat, luminance)` ; la couverture du groupe est
        /// l'**union** (`mask-composite:add`, initial) des
        /// alphas — `intersect`/`subtract`/`exclude` =
        /// résidus documentés.
        layers: Vec<MaskTile>,
    },
    /// Fin du groupe masqué — composite du tampon modulé
    /// par l'alpha du mask.
    PopMask,
    /// `clip-path:polygon(...)` (R-187) : les items jusqu'à
    /// `PopClip` ne se peignent que dans le polygone — mask
    /// `fill_path` au raster, même pile que `PushClip`.
    /// `inset`/`circle`/`ellipse` → `PushClip`/`PushClipRounded`
    /// (approximations rectangulaires documentées).
    PushClipPath {
        /// Sommets du polygone en coords document.
        points: Vec<(f32, f32)>,
    },
    /// `backdrop-filter` (R-M9b) : les pixels DÉJÀ peints
    /// sous `rect` reçoivent les ops, le contenu de la
    /// boîte se peint ensuite par-dessus.
    BackdropFilter {
        /// Zone filtrée (border box de l'élément).
        rect: Rect,
        /// Ops `<filter-function-list>` (mêmes que
        /// `filter` — `drop-shadow`/`url` hors scope).
        ops: Vec<FilterOp>,
    },
    /// Marqueur d'une boîte `display: list-item`
    /// (`list-style-type` ≠ `none`, issue #9) — forme
    /// primitive rasterisée directement (pas de shaping :
    /// `disc`/`circle`/`square`). Les marqueurs textuels
    /// (`decimal`, alpha, roman) sont hors portée — la
    /// display list ne résout pas les octets de fonte.
    Marker {
        /// Boîte englobante du glyphe de marqueur (zone dans
        /// laquelle la forme est inscrite).
        rect: Rect,
        /// Forme du marqueur.
        kind: MarkerKind,
        /// Encre (`color` hérité de l'élément `li`).
        color: Color,
    },
    /// Marqueur **textuel** (`decimal`, alpha, roman…) — R-108 :
    /// la display list ne shape pas, le raster résout la fonte et
    /// shape la chaîne au paint.
    MarkerText {
        /// Origine : x gauche, y de la baseline.
        origin: (f32, f32),
        /// Libellé (`"1."`, `"c."`, `"iv."`…).
        text: String,
        /// Requête fonte.
        font: explorer_layout::font::FontRequest,
        /// Encre (`color` hérité).
        color: Color,
        /// Taille utilisée (px).
        size_px: f32,
    },
}

/// Forme d'un marqueur de liste peint sans fonte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    /// `list-style-type: disc` — disque plein (défaut `ul`).
    Disc,
    /// `list-style-type: circle` — anneau.
    Circle,
    /// `list-style-type: square` — carré plein.
    Square,
}

/// Style de trait de bordure (sous-ensemble rasterisable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderStyle {
    /// Pas de trait.
    None,
    /// Plein.
    Solid,
    /// Pointillé (`dotted`/`dashed` — pattern au raster).
    Dotted,
    /// Double liseré.
    Double,
    /// Effet 3D enfoncé (R-166 : demi-teintes par côté).
    Inset,
    /// Effet 3D en relief (R-166 : demi-teintes par côté).
    Outset,
    /// Rainure 3D — moitié externe `inset`, interne `outset`.
    Groove,
    /// Crête 3D — moitié externe `outset`, interne `inset`.
    Ridge,
}

/// La frame prête à rasteriser : items + taille du viewport.
#[derive(Debug, Clone)]
pub struct DisplayList {
    /// Primitives dans l'ordre de peinture.
    pub items: Vec<DisplayItem>,
    /// Largeur du viewport (px).
    pub width: f32,
    /// Hauteur du document posé (px) — scrollable au-delà
    /// du viewport.
    pub height: f32,
    /// Correspondance boîte → plage d'items pour le build
    /// incrémental (R-37) — clé = chemin d'index enfants
    /// depuis la racine (stable aux re-mappages `NodeId`,
    /// conservateur aux insertions : un décalage produit
    /// un *miss*, jamais une copie fautive).
    pub spans: SpanMap,
    /// Epoch de l'arbre de boîtes peint (compteur côté
    /// appelant, bumpé au remplacement — la mutation
    /// in-place du chemin `relayout_subtree` conserve
    /// l'epoch). Égalité ⇒ fast-path pointeur autorisé.
    pub tree_epoch: u64,
    /// Émissions reportées sauté par la borne §B-26 (cycle
    /// de report cassé ou plafond total) — `0` = peinture
    /// saine. L'appelant le journalise (`explorer-render`
    /// ne trace pas — crate PIPELINE pur).
    pub dropped_emits: u32,
    /// Premier chemin ayant déclenché la borne §B-26 —
    /// diagnostic du cycle de report (chemins d'index
    /// enfants depuis la racine).
    pub first_cycle_path: Option<Vec<u16>>,
}

/// Répartition boîte→item d'une passe de peinture (§B-118 /
/// R-199 — mesure du collapse pluto.tv : 4 385 boîtes → 148
/// items, écran noir silencieux). Produite par les variantes
/// `*_stats` des builders — les builders ordinaires ne
/// comptent rien (zéro coût). `*_path` = chemin d'index
/// enfants du **premier** ancêtre fautif de sa catégorie :
/// le coupable se nomme en une lecture.
#[derive(Debug, Clone, Default)]
pub struct EmitStats {
    /// Boîtes visitées (entrées de `paint_box_impl`).
    pub boxes: u32,
    /// Items dans la liste finale.
    pub items: usize,
    /// Items par variante `DisplayItem` (nom → compte) —
    /// ce qui a **survécu** à la passe (ex. `Text` à zéro
    /// sur une page pleine = lignes jamais émises).
    pub by_kind: std::collections::BTreeMap<&'static str, u32>,
    /// Items dont la borne résolue (transforms composées)
    /// sort entièrement de la zone `[0,w]×[0,h]` — émis
    /// mais invisibles (translate géante, clip sortant).
    pub off_canvas: u32,
    /// Sous-arbres tués par `opacity <= 0` (retour précoce —
    /// contenu SPA gâté par opacité jusqu'à hydratation).
    pub drop_opacity0: u32,
    /// Sous-arbres tués par `backface-visibility:hidden` retournée.
    pub drop_backface: u32,
    /// Boîtes dont l'émission propre est sautée par
    /// `visibility:hidden|collapse` — **hérité** : un
    /// ancêtre hidden fait skipper chaque descendant.
    pub skip_visibility: u32,
    /// Reports (inline/positionnés/fixed) effectivement
    /// émis par un drain (`emit_scoped_at` ayant passé
    /// les bornes §B-26).
    pub deferred_emitted: u32,
    /// Reports sautés par la borne §B-26 (== `dropped_emits`).
    pub drop_cap: u32,
    /// Plages copiées depuis `prev` (build incrémental —
    /// un collapse sous ré-usage se lit ici).
    pub spans_copied: u32,
    /// Premier chemin tué par `opacity <= 0`.
    pub opacity0_path: Option<Vec<u16>>,
    /// Premier chemin tué par `backface-visibility`.
    pub backface_path: Option<Vec<u16>>,
    /// Premier chemin `visibility:hidden|collapse`.
    pub visibility_path: Option<Vec<u16>>,
    /// `(tx, ty)` de chaque `PushTransform` émis — une
    /// translation géante projette sa portée hors-canvas
    /// en une lecture (§B-118 phase fix).
    pub tf_offsets: Vec<(f32, f32)>,
    /// Borne résolue `(x, y, w, h)` du premier item
    /// hors-canvas.
    pub off_canvas_first: Option<(f32, f32, f32, f32)>,
    /// Transform composée `(tx, ty)` au premier item
    /// hors-canvas : ≈ identité = placement layout fautif,
    /// géante = `PushTransform` fautive.
    pub off_canvas_tf: Option<(f32, f32)>,
    /// Glyphes émis au total (tous items `Text`).
    pub glyphs: u32,
    /// Glyphes marqués `invisible` (`.notdef` PUA —
    /// avance conservée, rien peint) : ≈ `glyphs` =
    /// la fonte n'a pas servi les caractères (webfont
    /// absente/non décodée) → texte émis mais muet.
    pub invisible_glyphs: u32,
}

/// Porteur reporté conservé sous une plage ré-utilisable
/// (R-212) : chemin relatif depuis la boîte propriétaire +
/// clips `overflow` `(rect, radii)` applicables au drain.
type EscapedDefer = (Box<[u16]>, Box<[(Rect, [f32; 4])]>);

/// Clips `overflow` applicables à un porteur reporté, indexés
/// par son chemin — `(rect, radii)` par ancêtre clippé (R-212).
type DeferClips = std::collections::HashMap<Box<[u16]>, Box<[(Rect, [f32; 4])]>>;

/// Nom de variante pour la répartition [`EmitStats::by_kind`].
fn kind_name(it: &DisplayItem) -> &'static str {
    match it {
        DisplayItem::FillRect { .. } => "fill",
        DisplayItem::DecoRect { .. } => "deco",
        DisplayItem::BoxShadow { .. } => "shadow",
        DisplayItem::Border { .. } => "border",
        DisplayItem::Text { .. } => "text",
        DisplayItem::Placeholder { .. } => "placeholder",
        DisplayItem::Image { .. } => "image",
        DisplayItem::Emphasis { .. } => "emphasis",
        DisplayItem::BackgroundImage { .. } => "bg_image",
        DisplayItem::MaskedFill { .. } => "masked",
        DisplayItem::BorderImage { .. } => "border-image",
        DisplayItem::PushClip(_) => "push_clip",
        DisplayItem::PushClipRounded { .. } => "push_clip_r",
        DisplayItem::PopClip => "pop_clip",
        DisplayItem::PushTransform(_) => "push_tf",
        DisplayItem::PopTransform => "pop_tf",
        DisplayItem::PushFilter(_) => "push_filter",
        DisplayItem::PopFilter => "pop_filter",
        DisplayItem::PushBlend(_) => "push_blend",
        DisplayItem::PopBlend => "pop_blend",
        DisplayItem::PushMask { .. } => "push_mask",
        DisplayItem::PopMask => "pop_mask",
        DisplayItem::PushClipPath { .. } => "push_clippath",
        DisplayItem::BackdropFilter { .. } => "backdrop",
        DisplayItem::Marker { .. } => "marker",
        DisplayItem::MarkerText { .. } => "marker_text",
    }
}

/// Plage d'items émise par une boîte — indexe le re-usage
/// de `build_display_list_incremental` (R-37).
#[derive(Debug, Clone)]
pub struct SpanRec {
    /// Début de la plage dans `items`.
    pub start: u32,
    /// Fin de la plage (exclue).
    pub end: u32,
    /// `bx.rect` à l'émission (coords document, scroll exclu).
    pub rect: Rect,
    /// `off` utilisé à l'émission (`-scroll`, `(0,0)` sous
    /// `position: fixed`).
    pub off: (f32, f32),
    /// Fingerprint des champs peints — voir `box_sig`.
    pub sig: u64,
    /// `F_*` — restrictions de ré-usage.
    pub flags: u8,
    /// Adresse de la boîte à l'émission — fast-path quand
    /// l'arbre est **le même objet** (`tree_epoch` inchangé,
    /// mutation in-place du chemin `relayout_subtree`) :
    /// contrat — hors `{rect, node, lines, slot}`, aucun
    /// champ peint n'est muté in-place sur une boîte
    /// préservée (asserté en debug par `reuse_delta`).
    pub bx_ptr: usize,
    /// `bx.lines.as_ptr()` — `layout_block` réécrit `lines`
    /// (nouvelle allocation) sur les ancêtres re-posés :
    /// ptr différent ⇒ contenu inline potentiellement refait.
    pub lines_ptr: usize,
    /// `bx.children.as_ptr()` — le discriminant du fast-path :
    /// `*bx = fresh` (boîte sale remplacée par
    /// `relayout_subtree`) conserve l'adresse du *slot* mais
    /// le Vec enfants de `fresh` est alloué pendant que
    /// l'ancien vit encore — adresses disjointes garanties.
    /// `usize::MAX` quand `children` est vide : le ptr
    /// dangling ne discrimine pas — la feuille retombe alors
    /// sur `lines_ptr`, et les deux vides → signature.
    pub kids_ptr: usize,
    /// `bx.children.len()` — insertion d'enfants.
    pub kids_len: u32,
    /// Chemins des porteurs inline reportés au `defer`
    /// **hérité** pendant l'émission de la plage — relatifs
    /// à la boîte (préfixe = son chemin). Une copie plate
    /// les re-pousse dans le `defer` courant pour que le
    /// drain englobant les ré-émette (via leurs propres
    /// spans) — sinon le contenu en ligne disparaîtrait.
    /// Porteurs reportés capturés sous cette plage — chemin
    /// relatif + clips `overflow` applicables (R-212 : le
    /// report se re-clipe au drain).
    pub escaped: Box<[EscapedDefer]>,
}

/// Boîte `position: fixed` — sa plage n'est **pas**
/// translatée au scroll.
pub const F_FIXED: u8 = 1;
/// Ré-usage interdit (média remplacé dont le bitmap arrive
/// tard, `background-attachment: fixed` dont l'origine est
/// ancrée viewport) — la boîte se ré-émet toujours.
pub const F_NO_REUSE: u8 = 2;
/// La boîte a un enfant `position: sticky` — ses items
/// portent le `PushTransform` du décalage sticky, non
/// linéaire en scroll : toute translation non nulle
/// impose une ré-émission.
pub const F_STICKY_KID: u8 = 4;

impl DisplayList {
    /// Dump texte indenté — debug/golden sans raster.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for item in &self.items {
            use std::fmt::Write as _;
            match item {
                DisplayItem::FillRect {
                    rect,
                    color,
                    radii,
                    corners,
                    ..
                } => {
                    let _ = writeln!(out, "fill {rect:?} #{color:?} r{radii:?} c{corners:?}");
                }
                DisplayItem::DecoRect {
                    rect, style, color, ..
                } => {
                    let _ = writeln!(out, "deco s{style} #{color:?} {rect:?}");
                }
                DisplayItem::Border {
                    rect,
                    widths,
                    colors,
                    styles,
                    radii,
                    corners,
                    ..
                } => {
                    let _ = writeln!(
                        out,
                        "border {rect:?} w{widths:?} c{colors:?} s{styles:?} r{radii:?} c{corners:?}"
                    );
                }
                DisplayItem::Text {
                    origin,
                    run,
                    color,
                    font,
                    ..
                } => {
                    let gid_sum: u32 = run.glyphs.iter().map(|g| u32::from(g.id)).sum();
                    let _ = writeln!(
                        out,
                        "text @{:.1},{:.1} ({} glyphs g{gid_sum} adv{:.2}) #{color:?} f[{},{},i{},c{},s{},k{},ft{:?}]",
                        origin.0,
                        origin.1,
                        run.glyphs.len(),
                        run.advance,
                        font.families.join("|"),
                        font.weight,
                        u8::from(font.italic),
                        font.caps,
                        font.stretch,
                        u8::from(font.kern_off),
                        font.feature_settings,
                    );
                }
                DisplayItem::Placeholder { rect, label } => {
                    let _ = writeln!(out, "placeholder {label} {rect:?}");
                }
                DisplayItem::Image { rect, img } => {
                    let _ = writeln!(out, "image {}x{} {rect:?}", img.width, img.height);
                }
                DisplayItem::BackgroundImage {
                    rect,
                    img,
                    repeat,
                    origin,
                    blend,
                } => {
                    let _ = writeln!(
                        out,
                        "bg-image {}x{} {rect:?} r{repeat:?} o{origin:?} b{blend}",
                        img.width, img.height
                    );
                }
                DisplayItem::MaskedFill { rect, mask, .. } => {
                    let _ = writeln!(out, "masked-fill {}x{} {rect:?}", mask.width, mask.height);
                }
                DisplayItem::BorderImage { rect, img, .. } => {
                    let _ = writeln!(out, "border-image {}x{} {rect:?}", img.width, img.height);
                }
                DisplayItem::BoxShadow { rect, .. } => {
                    let _ = writeln!(out, "box-shadow {rect:?}");
                }
                DisplayItem::Marker { rect, kind, .. } => {
                    let _ = writeln!(out, "marker {kind:?} {rect:?}");
                }
                DisplayItem::MarkerText { origin, text, .. } => {
                    let _ = writeln!(out, "marker-text {text:?} {origin:?}");
                }
                DisplayItem::PushClip(r) => {
                    let _ = writeln!(out, "push-clip {r:?}");
                }
                DisplayItem::PushClipRounded { rect, radii } => {
                    let _ = writeln!(out, "push-clip-rounded {rect:?} {radii:?}");
                }
                DisplayItem::PopClip => out.push_str("pop-clip\n"),
                DisplayItem::PushTransform(m) => {
                    let _ = writeln!(out, "push-transform {m:?}");
                }
                DisplayItem::PopTransform => out.push_str("pop-transform\n"),
                DisplayItem::PushFilter(ops) => {
                    let _ = writeln!(out, "push-filter {ops:?}");
                }
                DisplayItem::PopFilter => out.push_str("pop-filter\n"),
                DisplayItem::PushBlend(b) => {
                    let _ = writeln!(out, "push-blend {b}");
                }
                DisplayItem::PushClipPath { points } => {
                    let _ = writeln!(out, "push-clippath {}", points.len());
                }
                DisplayItem::Emphasis { origin, kind, .. } => {
                    let _ = writeln!(out, "emphasis k{kind} @{origin:?}");
                }
                DisplayItem::PopBlend => out.push_str("pop-blend\n"),
                DisplayItem::PushMask { rect, layers, .. } => {
                    let _ = write!(out, "push-mask x{} {rect:?}", layers.len());
                    for l in layers {
                        let _ = write!(
                            out,
                            " [d{dst:?} r{repeat:?} l{lumi} c{comp}]",
                            dst = l.dst,
                            repeat = l.repeat,
                            lumi = l.luminance,
                            comp = l.composite
                        );
                    }
                    out.push('\n');
                }
                DisplayItem::PopMask => out.push_str("pop-mask\n"),
                DisplayItem::BackdropFilter { rect, .. } => {
                    let _ = writeln!(out, "backdrop-filter {rect:?}");
                }
            }
        }
        out
    }

    /// Régions à repeindre entre `prev` et `self` — dirty-rects
    /// (R-7). Chaque item est résolu dans son contexte de peinture
    /// (pile `PushTransform`/`PushClip`, même ordre
    /// `pre_concat` que le raster) ; quand un item diffère — ou
    /// que sa zone résolue a bougé — les zones ancienne **et**
    /// nouvelle sont rapportées. Les rects retournés sont dans
    /// l'espace framebuffer (scroll déjà cuit), fusionnés par
    /// intersection ; à l'appelant d'intersecter sa cible.
    /// Listes identiques → `vec![]` (frame propre, rien à
    /// soumettre).
    #[must_use]
    pub fn changed_rects(&self, prev: &DisplayList) -> Vec<Rect> {
        let (old, new) = (resolved_bounds(&prev.items), resolved_bounds(&self.items));
        let mut out = Vec::new();
        for i in 0..self.items.len().max(prev.items.len()) {
            let same = match (self.items.get(i), prev.items.get(i)) {
                (Some(a), Some(b)) => item_eq(a, b) && new[i] == old[i],
                (None, None) => true,
                _ => false,
            };
            if !same {
                if let Some(r) = old.get(i).copied().flatten() {
                    out.push(r);
                }
                if let Some(r) = new.get(i).copied().flatten() {
                    out.push(r);
                }
            }
        }
        merge_rects(&mut out);
        out
    }
}

/// Zone peinte de chaque item, transform/clip résolus — `None`
/// pour les items d'état (`Push*/Pop*`). Rejoue la même
/// sémantique de pile que `rasterize` (transforms en
/// `pre_concat`, clips poussés dans leur transform courant).
/// `pub(crate)` : `rasterize_dirty` (R-37) filtre les items
/// par intersection avec la fenêtre sale.
pub(crate) fn resolved_bounds(items: &[DisplayItem]) -> Vec<Option<Rect>> {
    let mut tfs: Vec<tiny_skia::Transform> = Vec::new();
    let mut clips: Vec<Rect> = Vec::new();
    let mut cur = tiny_skia::Transform::default();
    items
        .iter()
        .map(|item| match item {
            DisplayItem::PushTransform(m) => {
                tfs.push(*m);
                cur = tfs
                    .iter()
                    .fold(tiny_skia::Transform::default(), |acc, t| acc.pre_concat(*t));
                None
            }
            DisplayItem::PopTransform => {
                tfs.pop();
                cur = tfs
                    .iter()
                    .fold(tiny_skia::Transform::default(), |acc, t| acc.pre_concat(*t));
                None
            }
            DisplayItem::PushClip(r) => {
                clips.push(apply_tf(*r, cur));
                None
            }
            DisplayItem::PopClip => {
                clips.pop();
                None
            }
            _ => item_bounds(item).map(|r| {
                let r = apply_tf(r, cur);
                clips
                    .iter()
                    .fold(r, |a, c| rect_intersect(a, *c).unwrap_or(a))
            }),
        })
        .collect()
}

/// Zone couverte par un item de dessin, avant transform/clip.
#[allow(clippy::cast_precision_loss)] // longueurs de texte → f32
fn item_bounds(item: &DisplayItem) -> Option<Rect> {
    match item {
        DisplayItem::FillRect { rect, .. }
        | DisplayItem::DecoRect { rect, .. }
        | DisplayItem::Border { rect, .. }
        | DisplayItem::Placeholder { rect, .. }
        | DisplayItem::Image { rect, .. }
        | DisplayItem::MaskedFill { rect, .. }
        | DisplayItem::BorderImage { rect, .. }
        | DisplayItem::Marker { rect, .. }
        | DisplayItem::BackdropFilter { rect, .. }
        | DisplayItem::BackgroundImage { rect, .. } => Some(*rect),
        // L'ombre déborde sa boîte (offset + spread +
        // blur) — borne propre pour le dirty-tracking.
        DisplayItem::BoxShadow {
            rect,
            offset,
            blur,
            spread,
            ..
        } => {
            let g = blur + spread.max(0.0);
            Some(Rect {
                x: rect.x + offset.0 - g,
                y: rect.y + offset.1 - g,
                width: rect.width + 2.0 * g,
                height: rect.height + 2.0 * g,
            })
        }
        // Pas de fonte résolue ici — borne approchée :
        // ~0.6em/caractère de large, 1.2em de haut au-dessus de
        // la baseline.
        DisplayItem::MarkerText {
            origin,
            text,
            size_px,
            ..
        } => Some(Rect {
            x: origin.0,
            y: origin.1 - size_px * 1.2,
            width: text.chars().count() as f32 * size_px * 0.6,
            height: size_px * 1.2,
        }),
        DisplayItem::Text { origin, run, .. } => Some(match run.ink {
            // R-37 : bbox d'encre réelle (glyphes) plutôt que
            // la boîte de métriques — couvre l'overhang
            // (italique, accents, plein-em) sans inflation.
            // Repli boîte métrique si le run n'a pas de
            // contours (espaces) ou fonte sans bbox.
            Some(ink) => Rect {
                x: origin.0 + ink.x,
                y: origin.1 + ink.y,
                width: ink.width,
                height: ink.height,
            },
            None => Rect {
                x: origin.0,
                y: origin.1 - run.metrics.ascent,
                width: run.advance,
                height: run.metrics.ascent - run.metrics.descent,
            },
        }),
        DisplayItem::PushClip(_)
        | DisplayItem::PushClipRounded { .. }
        | DisplayItem::PopClip
        | DisplayItem::PushClipPath { .. }
        | DisplayItem::PushTransform(_)
        | DisplayItem::PopTransform
        | DisplayItem::PushFilter(_)
        | DisplayItem::PopFilter
        | DisplayItem::PushBlend(_)
        | DisplayItem::PopBlend
        | DisplayItem::PushMask { .. }
        | DisplayItem::PopMask => None,
        // Marques de texte — borne du run (avance × taille).
        DisplayItem::Emphasis {
            origin,
            run,
            size_px,
            ..
        } => Some(Rect {
            x: origin.0,
            y: origin.1 - size_px * 1.2,
            width: run.advance,
            height: size_px * 2.0,
        }),
    }
}

/// Applique une matrice affine à un `Rect` — AABB des quatre
/// coins transformés (la rotation élargit, comme le raster).
fn apply_tf(r: Rect, t: tiny_skia::Transform) -> Rect {
    let (mut lx, mut ly, mut hx, mut hy) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for (cx, cy) in [
        (r.x, r.y),
        (r.x + r.width, r.y),
        (r.x, r.y + r.height),
        (r.x + r.width, r.y + r.height),
    ] {
        let (px, py) = (t.sx * cx + t.kx * cy + t.tx, t.ky * cx + t.sy * cy + t.ty);
        lx = lx.min(px);
        ly = ly.min(py);
        hx = hx.max(px);
        hy = hy.max(py);
    }
    Rect {
        x: lx,
        y: ly,
        width: hx - lx,
        height: hy - ly,
    }
}

/// Égalité visuelle de deux items — `Arc` comparés par
/// identité (pixels différents = arc différent ⇒ dirty).
#[allow(clippy::too_many_lines)]
fn item_eq(a: &DisplayItem, b: &DisplayItem) -> bool {
    use DisplayItem as D;
    match (a, b) {
        (
            D::BoxShadow {
                rect: r1,
                color: c1,
                ..
            },
            D::BoxShadow {
                rect: r2,
                color: c2,
                ..
            },
        ) => r1 == r2 && c1 == c2,
        (
            D::FillRect {
                rect: r1,
                color: c1,
                radii: ra1,
                corners: k1,
            },
            D::FillRect {
                rect: r2,
                color: c2,
                radii: ra2,
                corners: k2,
            },
        ) => r1 == r2 && c1 == c2 && ra1 == ra2 && k1 == k2,
        (
            D::DecoRect {
                rect: r1,
                color: c1,
                style: s1,
            },
            D::DecoRect {
                rect: r2,
                color: c2,
                style: s2,
            },
        ) => r1 == r2 && c1 == c2 && s1 == s2,
        (
            D::Border {
                rect: r1,
                widths: w1,
                colors: c1,
                styles: s1,
                radii: ra1,
                corners: k1,
            },
            D::Border {
                rect: r2,
                widths: w2,
                colors: c2,
                styles: s2,
                radii: ra2,
                corners: k2,
            },
        ) => r1 == r2 && w1 == w2 && c1 == c2 && s1 == s2 && ra1 == ra2 && k1 == k2,
        (
            D::Text {
                origin: o1,
                run: ru1,
                font: f1,
                color: c1,
                size_px: p1,
            },
            D::Text {
                origin: o2,
                run: ru2,
                font: f2,
                color: c2,
                size_px: p2,
            },
        ) => o1 == o2 && ru1 == ru2 && f1 == f2 && c1 == c2 && p1 == p2,
        (
            D::Placeholder {
                rect: r1,
                label: l1,
            },
            D::Placeholder {
                rect: r2,
                label: l2,
            },
        ) => r1 == r2 && l1 == l2,
        (D::Image { rect: r1, img: i1 }, D::Image { rect: r2, img: i2 }) => {
            r1 == r2 && std::sync::Arc::ptr_eq(i1, i2)
        }
        (
            D::BackgroundImage {
                rect: r1,
                img: i1,
                repeat: p1,
                origin: o1,
                blend: b1,
            },
            D::BackgroundImage {
                rect: r2,
                img: i2,
                repeat: p2,
                origin: o2,
                blend: b2,
            },
        ) => r1 == r2 && p1 == p2 && o1 == o2 && b1 == b2 && std::sync::Arc::ptr_eq(i1, i2),
        (
            D::MaskedFill {
                rect: r1,
                color: c1,
                mask: m1,
                dst: d1,
                repeat: p1,
                luminance: u1,
            },
            D::MaskedFill {
                rect: r2,
                color: c2,
                mask: m2,
                dst: d2,
                repeat: p2,
                luminance: u2,
            },
        ) => {
            r1 == r2
                && c1 == c2
                && d1 == d2
                && p1 == p2
                && u1 == u2
                && std::sync::Arc::ptr_eq(m1, m2)
        }
        (
            D::BorderImage {
                rect: r1,
                img: i1,
                src_edges: s1,
                dst_edges: d1,
                fill_center: f1,
                repeat: p1,
            },
            D::BorderImage {
                rect: r2,
                img: i2,
                src_edges: s2,
                dst_edges: d2,
                fill_center: f2,
                repeat: p2,
            },
        ) => {
            r1 == r2
                && s1 == s2
                && d1 == d2
                && f1 == f2
                && p1 == p2
                && std::sync::Arc::ptr_eq(i1, i2)
        }
        (
            D::Marker {
                rect: r1,
                kind: k1,
                color: c1,
            },
            D::Marker {
                rect: r2,
                kind: k2,
                color: c2,
            },
        ) => r1 == r2 && k1 == k2 && c1 == c2,
        (
            D::MarkerText {
                origin: o1,
                text: t1,
                font: f1,
                color: c1,
                size_px: s1,
            },
            D::MarkerText {
                origin: o2,
                text: t2,
                font: f2,
                color: c2,
                size_px: s2,
            },
        ) => o1 == o2 && t1 == t2 && f1 == f2 && c1 == c2 && s1 == s2,
        (D::PushClip(r1), D::PushClip(r2)) => r1 == r2,
        (D::PushClipPath { points: p1 }, D::PushClipPath { points: p2 }) => p1 == p2,
        (
            D::PushClipRounded {
                rect: r1,
                radii: a1,
            },
            D::PushClipRounded {
                rect: r2,
                radii: a2,
            },
        ) => r1 == r2 && a1 == a2,
        (D::PushFilter(o1), D::PushFilter(o2)) => o1 == o2,
        (D::PushBlend(b1), D::PushBlend(b2)) => b1 == b2,
        (D::PopBlend, D::PopBlend) => true,
        (
            D::Emphasis {
                origin: o1,
                run: r1,
                size_px: s1,
                kind: k1,
                color: c1,
                over: v1,
            },
            D::Emphasis {
                origin: o2,
                run: r2,
                size_px: s2,
                kind: k2,
                color: c2,
                over: v2,
            },
        ) => {
            o1 == o2
                && std::sync::Arc::ptr_eq(r1, r2)
                && s1 == s2
                && k1 == k2
                && c1 == c2
                && v1 == v2
        }
        (D::BackdropFilter { rect: r1, ops: o1 }, D::BackdropFilter { rect: r2, ops: o2 }) => {
            r1 == r2 && o1 == o2
        }
        (D::PushTransform(m1), D::PushTransform(m2)) => {
            (m1.sx, m1.ky, m1.kx, m1.sy, m1.tx, m1.ty) == (m2.sx, m2.ky, m2.kx, m2.sy, m2.tx, m2.ty)
        }
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

/// Intersection de deux rects — `None` si disjoints.
fn rect_intersect(a: Rect, b: Rect) -> Option<Rect> {
    let (x1, y1) = (a.x.max(b.x), a.y.max(b.y));
    let (x2, y2) = (
        (a.x + a.width).min(b.x + b.width),
        (a.y + a.height).min(b.y + b.height),
    );
    (x2 > x1 && y2 > y1).then_some(Rect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    })
}

/// Union englobante de deux rects.
fn rect_union(a: Rect, b: Rect) -> Rect {
    let (x1, y1) = (a.x.min(b.x), a.y.min(b.y));
    let (x2, y2) = (
        (a.x + a.width).max(b.x + b.width),
        (a.y + a.height).max(b.y + b.height),
    );
    Rect {
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    }
}

/// Fusionne les rects qui se chevauchent — balayage trié
/// par `x` (un rect ne se compare qu'aux composantes encore
/// ouvertes horizontalement) puis consolidation bornée en
/// point fixe. O(k log k + k·ouvertes + C²·passes) avec
/// C ≪ k, au lieu du O(k²) historique (mesuré ~2 s sur
/// ~8 500 rects d'un diff dégénéré, dette journal NAV 55g).
/// Couverture identique ; la partition peut fusionner
/// davantage que la passe unique d'avant (qui n'atteignait
/// pas le point fixe qu'elle annonçait).
fn merge_rects(rects: &mut Vec<Rect>) {
    if rects.len() < 2 {
        return;
    }
    let mut order = std::mem::take(rects);
    order.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    let mut comps: Vec<Rect> = Vec::new();
    // Composantes **fermées** (`c.x + c.width < r.x` — aucun
    // rect futur ne peut les toucher, le tri par x le
    // garantit) : la compaction les sort du balayage mais
    // elles doivent rejoindre la SORTIE — avant, `retain`
    // les supprimait purement, leurs zones n'entraient
    // jamais dans le damage → pixels changés jamais
    // repeints (issue #11 bis, couture NAV : diff
    // quasi-total lobste.rs, in=2060 out=39 au lieu de ~505).
    let mut done: Vec<Rect> = Vec::new();
    for r in order {
        let mut hits: Vec<usize> = Vec::new();
        let mut dead = 0usize;
        for (i, c) in comps.iter().enumerate() {
            if c.x + c.width < r.x {
                // Fermée : aucun rect futur (triés par x) ne
                // peut la toucher — marquée pour compaction.
                dead += 1;
                continue;
            }
            if r.y < c.y + c.height && c.y < r.y + r.height {
                hits.push(i);
            }
        }
        match hits.split_first() {
            None => comps.push(r),
            Some((&first, rest)) => {
                let mut bb = rect_union(comps[first], r);
                // `r` relie plusieurs composantes → elles
                // fusionnent dans `first` (retrait en index
                // décroissant : les suivants restent valides).
                for &i in rest.iter().rev() {
                    bb = rect_union(bb, comps[i]);
                    comps[i] = comps[comps.len() - 1];
                    comps.pop();
                }
                comps[first] = bb;
            }
        }
        // Compaction amortie : sort du balayage les
        // composantes fermées quand elles dominent — borne
        // le travail courant. Elles rejoignent `done`
        // (réémises en sortie), pas le néant.
        if comps.len() > 64 && dead * 2 >= comps.len() {
            comps.retain(|c| {
                if c.x + c.width >= r.x {
                    true
                } else {
                    done.push(*c);
                    false
                }
            });
        }
    }
    // Les fermées ne peuvent plus fusionner entre elles (le
    // tri le garantissait déjà) mais une composante vivante
    // peut avoir grossi jusque dans leur zone : réinjectées
    // avant la consolidation, le point fixe les absorbe.
    comps.extend(done);
    // Consolidation : une composante qui a grossi vers la
    // gauche peut encore chevaucher une voisine fermée.
    // Point fixe borné sur les C ≪ k survivantes.
    for _ in 0..4 {
        let mut merged = false;
        let mut i = 0;
        while i < comps.len() {
            let mut j = i + 1;
            while j < comps.len() {
                if rect_intersect(comps[i], comps[j]).is_some() {
                    comps[i] = rect_union(comps[i], comps[j]);
                    comps.swap_remove(j);
                    merged = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged {
            break;
        }
    }
    *rects = comps;
}

/// Construit la display list d'un arbre posé — pur,
/// déterministe, borné par la taille de l'arbre. `height`
/// couvre au moins le viewport. `scroll_y` décale tout le
/// contenu vers le haut, **sauf** les sous-arbres
/// `position: fixed` qui restent ancrés au viewport.
/// `images` = pixels décodés par `NodeId` `<img>` — une
/// boîte remplacée (`intrinsic` posé) peint son bitmap.
/// [`build_display_list`] équivaut à une map vide.
///
/// L'arbre posé d'un document **en construction** est une entrée
/// supportée (R-1) : la liste reflète l'instantané `&LayoutBox`
/// reçu — jamais l'arbre DOM vivant — donc une frame peinte
/// reste cohérente quand le parse reprend ensuite.
#[must_use]
pub fn build_display_list_imaged(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    images: &crate::ImageTable,
) -> DisplayList {
    build_display_list_full(
        root,
        viewport_w,
        viewport_h,
        scroll_y,
        &crate::PaintImages::from_replaced(images.clone()),
    )
}

/// Variante complète : `images` porte les `<img>` décodés ET
/// les `background-image` résolus (par `NodeId`). [`build_display_list`]
/// équivaut à des tables vides.
#[must_use]
pub fn build_display_list_full(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
    images: &crate::PaintImages,
) -> DisplayList {
    build_display_list_scrolled(root, viewport_w, viewport_h, (0.0, scroll_y), images)
}

/// Variante à scroll horizontal : `scroll = (x, y)` — les
/// seuils sticky `left`/`right` le consomment (R-6).
#[must_use]
pub fn build_display_list_scrolled(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll: (f32, f32),
    images: &crate::PaintImages,
) -> DisplayList {
    build_list(root, viewport_w, viewport_h, scroll, images, None, 0, None)
}

/// Variante de mesure (§B-118 / R-199) : renvoie la liste +
/// la répartition boîte→item [`EmitStats`] — comptage opt-in,
/// zéro coût sur les builders ordinaires.
#[must_use]
pub fn build_display_list_scrolled_stats(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll: (f32, f32),
    images: &crate::PaintImages,
) -> (DisplayList, EmitStats) {
    let mut st = EmitStats::default();
    let list = build_list(
        root,
        viewport_w,
        viewport_h,
        scroll,
        images,
        None,
        0,
        Some(&mut st),
    );
    (list, st)
}

/// Variante incrémentale (R-37) : `prev` fournit les plages
/// de la peinture précédente — les sous-arbres inchangés
/// (même `sig`, même taille) sont **copiés** au lieu d'être
/// ré-émis, éventuellement translatés. `prev` ignorée si la
/// largeur diffère (les coords viewport-ancrées bougent).
///
/// Correction du ré-usage : une plage n'est copiée que si
/// **tous** les descendants partagent le même Δ (translation
/// rigide — un `fixed` interne a Δ=0 et fait descendre le
/// parent) ; contenu/style/lignes sont fingerprintés par
/// `box_sig`. L'équivalence « liste incrémentale == liste
/// pleine » est assertée par `tests/dirty_raster.rs`.
#[must_use]
pub fn build_display_list_incremental(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll: (f32, f32),
    images: &crate::PaintImages,
    prev: Option<&DisplayList>,
    tree_epoch: u64,
) -> DisplayList {
    build_list(
        root, viewport_w, viewport_h, scroll, images, prev, tree_epoch, None,
    )
}

/// Variante incrémentale de mesure (§B-118) — mêmes compteurs
/// que [`build_display_list_scrolled_stats`], `spans_copied`
/// isole les plages ré-utilisées depuis `prev`.
#[must_use]
pub fn build_display_list_incremental_stats(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll: (f32, f32),
    images: &crate::PaintImages,
    prev: Option<&DisplayList>,
    tree_epoch: u64,
) -> (DisplayList, EmitStats) {
    let mut st = EmitStats::default();
    let list = build_list(
        root,
        viewport_w,
        viewport_h,
        scroll,
        images,
        prev,
        tree_epoch,
        Some(&mut st),
    );
    (list, st)
}

/// §14.2 : émet le fond du canvas (couleur puis image) et
/// renvoie le `NodeId` de la boîte source — racine si elle a
/// un fond, sinon son premier enfant bloc (`body`) — pour que
/// `paint_self` ne le repeigne pas sur sa boîte propre.
/// `canvas` = rect de peinture (viewport × hauteur document).
#[allow(clippy::cast_precision_loss)] // dims image ≪ 2^23
fn emit_canvas_bg(
    root: &LayoutBox,
    canvas: Rect,
    viewport_w: f32,
    viewport_h: f32,
    images: &crate::PaintImages,
    items: &mut Vec<DisplayItem>,
) -> Option<explorer_dom::NodeId> {
    let (canvas_from_root, canvas_bx) = if has_bg(&root.style) {
        (true, Some(root))
    } else {
        (
            false,
            root.children
                .iter()
                .find(|c| c.kind == BoxKind::Block && has_bg(&c.style)),
        )
    };
    let src = canvas_bx?;
    // R-265 — `currentcolor` résolu via `st.color`.
    if let Some(c) = src.style.color("background-color") {
        if c.a > 0 {
            items.push(DisplayItem::FillRect {
                rect: canvas,
                color: c,
                radii: [0.0; 4],
                corners: [1.0; 4],
            });
        }
    }
    // Origine des tuiles : padding box de la racine pour SON
    // fond (le `0 -1em` de `margin-collapse-020` se mesure
    // depuis son propre bord) ; canvas pour le fond propagé
    // de `body`.
    let (area, base) = if canvas_from_root {
        let bl = border_side_w(&src.style, "left");
        let bt = border_side_w(&src.style, "top");
        let br = border_side_w(&src.style, "right");
        let bb = border_side_w(&src.style, "bottom");
        (
            (
                (src.rect.width - bl - br).max(0.0),
                (src.rect.height - bt - bb).max(0.0),
            ),
            (src.rect.x + bl, src.rect.y + bt),
        )
    } else {
        ((viewport_w, viewport_h), (0.0, 0.0))
    };
    emit_bg_layers(items, src, images, canvas, area, base, (0.0, 0.0));
    src.node
}

/// Corps partagé des builders — `prev = None` : build plein.
#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
fn build_list(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll: (f32, f32),
    images: &crate::PaintImages,
    prev: Option<&DisplayList>,
    tree_epoch: u64,
    stats: Option<&mut EmitStats>,
) -> DisplayList {
    let mut items = Vec::new();
    let mut spans = SpanMap::default();
    let same_tree = prev.is_some_and(|p| p.tree_epoch == tree_epoch);
    let inc = prev
        .filter(|p| p.width.to_bits() == viewport_w.to_bits())
        .map(|p| IncCx {
            items: &p.items,
            spans: &p.spans,
        });
    let mut cx = PaintCx {
        vp: (viewport_w, viewport_h),
        images,
        spans: &mut spans,
        path: Vec::new(),
        inc,
        same_tree,
        canvas_bg_src: None,
        emit_per_path: std::collections::HashMap::new(),
        sticky_deltas: std::collections::HashMap::new(),
        offset_tf: std::collections::HashMap::new(),
        emits_total: 0,
        dropped: 0,
        first_cycle_path: None,
        stats,
        us_copy: 0,
        us_book: 0,
        n_reuse: 0,
        n_paint: 0,
        n_culled: 0,
        clip_stack: Vec::new(),
        pos_floor: 0,
        defer_clips: DeferClips::new(),
    };
    // §14.2 : le fond du canvas vient de la racine ; si elle
    // n'en a pas (couleur transparente et pas d'image), c'est
    // celui de son premier enfant bloc (`body`) qui propage —
    // ancré à l'origine du canvas alors que le fond de la
    // racine s'ancre à sa propre padding box
    // (`margin-collapse-021` : le fond de `body` positionné
    // `no-repeat` part du coin du canvas, pas de sa boîte
    // décalée par sa marge).
    // Hauteur de la liste = région défilante réelle : le
    // contenu qui déborde un ancêtre à hauteur spécifiée
    // (`html,body{height:100%}` vector-2022, R-82) doit
    // entrer dans la frame — `rect.height` seul coupait tout.
    let doc_h = scrollable_overflow_bottom(root).max(viewport_h);
    let canvas = Rect {
        x: 0.0,
        y: 0.0,
        width: viewport_w,
        height: doc_h,
    };
    cx.canvas_bg_src = emit_canvas_bg(root, canvas, viewport_w, viewport_h, images, &mut items);
    // Portées racine des reports : inlines (App. E étape 5)
    // et positionnés `z ≥ 0` (étapes 8-9 — la racine ferme
    // toujours le dernier contexte d'empilement).
    let mut defer = Vec::new();
    let mut pos_defer = Vec::new();
    // `fixed` : file racine dédiée — drainée APRÈS toute la
    // descente, hors de tout clip `overflow` ancêtre (leur
    // bloc conteneur est le viewport). Fusionnée par z dans
    // `pos_defer` avant le drain pour garder l'ordre global.
    let mut fixed_defer = Vec::new();
    paint_box(
        root,
        &mut items,
        (-scroll.0, -scroll.1),
        &mut cx,
        &mut defer,
        &mut pos_defer,
        &mut fixed_defer,
    );
    {
        // Span racine — `escaped` = les porteurs poussés aux
        // files racine (chemins absolus = relatifs à la racine).
        let mut esc = escaped_paths(&defer, 0, &cx.path, &cx).into_vec();
        esc.extend(escaped_paths(&pos_defer, 0, &cx.path, &cx).into_vec());
        esc.extend(escaped_paths(&fixed_defer, 0, &cx.path, &cx).into_vec());
        let end = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let sig = box_sig(root);
        cx.record_span(
            root,
            0,
            end,
            (-scroll.0, -scroll.1),
            sig,
            esc.into_boxed_slice(),
        );
    }
    let mut i = 0;
    while i < defer.len() {
        let (p, c) = defer[i].clone();
        i += 1;
        emit_scoped_at(
            p,
            c,
            &mut items,
            (-scroll.0, -scroll.1),
            &mut cx,
            &mut pos_defer,
            &mut fixed_defer,
        );
    }
    // `fixed` remontés de tout le sous-arbre → fusion triée
    // par z dans `pos_defer` (ordre de peinture racine). La
    // fusion se refait à chaque itération : l'émission d'un
    // positionné peut à son tour remonter des `fixed`
    // (descendants fixés d'un absolu…).
    for (p, c) in std::mem::take(&mut fixed_defer) {
        push_z(&mut pos_defer, 0, p, c);
    }
    // Positionnés reportés — déjà triés stable par z (`push_z`).
    // `pending` absorbe les porteurs poussés pendant l'émission
    // (positionnés/fixed descendants découverts en peignant un
    // différé, ou `escaped` re-poussés par une copie de span) :
    // ils rejoignent `pos_defer` **au-delà du curseur** — triés
    // parmi les non-lues. Trier sur la file entière compterait
    // le préfixe déjà drainé : une entrée fusionnée sous le
    // curseur ne serait jamais drainée, et l'entrée déjà émise
    // qu'elle déplace refait surface indéfiniment (cycle de
    // report §B-26).
    let mut pending: Vec<(Vec<u16>, &LayoutBox)> = Vec::new();
    let mut i = 0;
    while i < pos_defer.len() {
        let (p, c) = pos_defer[i].clone();
        i += 1;
        emit_scoped_at(
            p,
            c,
            &mut items,
            (-scroll.0, -scroll.1),
            &mut cx,
            &mut pending,
            &mut fixed_defer,
        );
        for (p, c) in std::mem::take(&mut fixed_defer) {
            push_z(&mut pos_defer, i, p, c);
        }
        for (p, c) in std::mem::take(&mut pending) {
            push_z(&mut pos_defer, i, p, c);
        }
    }
    let (dropped_emits, first_cycle_path) = (cx.dropped, cx.first_cycle_path.take());
    if let Some(st) = cx.stats {
        st.items = items.len();
        st.drop_cap = dropped_emits;
        // Post-pass : répartition par variante + bornes
        // résolues (transforms composées) — un item émis hors
        // de la zone canvas est peint mais invisible. La pile
        // `PushTransform` est rejouée en miroir pour lire la
        // transform courante au premier item hors-zone (§B-118).
        let mut tfs: Vec<tiny_skia::Transform> = Vec::new();
        let mut cur = tiny_skia::Transform::default();
        for (it, b) in items.iter().zip(resolved_bounds(&items)) {
            *st.by_kind.entry(kind_name(it)).or_insert(0) += 1;
            match it {
                DisplayItem::PushTransform(m) => {
                    tfs.push(*m);
                    cur = tfs
                        .iter()
                        .fold(tiny_skia::Transform::default(), |acc, t| acc.pre_concat(*t));
                    st.tf_offsets.push((m.tx, m.ty));
                }
                DisplayItem::PopTransform => {
                    tfs.pop();
                    cur = tfs
                        .iter()
                        .fold(tiny_skia::Transform::default(), |acc, t| acc.pre_concat(*t));
                }
                DisplayItem::Text { run, .. } => {
                    // Compteurs indicatifs : la saturation
                    // u32::MAX est fine (jamais atteinte).
                    st.glyphs = st
                        .glyphs
                        .saturating_add(u32::try_from(run.glyphs.len()).unwrap_or(u32::MAX));
                    st.invisible_glyphs = st.invisible_glyphs.saturating_add(
                        u32::try_from(run.glyphs.iter().filter(|g| g.invisible).count())
                            .unwrap_or(u32::MAX),
                    );
                }
                _ => {}
            }
            if b.is_some_and(|r| {
                !(r.x < viewport_w && r.y < doc_h && r.x + r.width > 0.0 && r.y + r.height > 0.0)
            }) {
                st.off_canvas += 1;
                if st.off_canvas_first.is_none() {
                    st.off_canvas_first = b.map(|r| (r.x, r.y, r.width, r.height));
                    st.off_canvas_tf = Some((cur.tx, cur.ty));
                }
            }
        }
    }
    tracing::info!(
        target: "sync_profile",
        tab = 0u64,
        phase = "emit.parts",
        reuse = cx.n_reuse,
        painted = cx.n_paint,
        culled = cx.n_culled,
        copy_us = cx.us_copy,
        book_us = cx.us_book,
        "phase sync"
    );
    DisplayList {
        items,
        width: viewport_w,
        height: doc_h,
        spans,
        tree_epoch,
        dropped_emits,
        first_cycle_path,
    }
}

/// Contexte d'une passe de peinture — viewport, images,
/// collecteur de `spans`, chemin courant (clé des spans)
/// et source de ré-usage incrémental éventuelle (R-37).
struct PaintCx<'a> {
    /// Viewport `(w, h)` — consommé par sticky et les fonds
    /// `background-attachment: fixed`.
    vp: (f32, f32),
    /// Bitmaps par `NodeId` (remplacés, fonds, svg, canvas).
    images: &'a crate::PaintImages,
    /// Spans produits par cette passe.
    spans: &'a mut SpanMap,
    /// Chemin d'index-enfants de la boîte courante — la clé
    /// de span (un `u16` par niveau, saturé — l'overflow est
    /// un miss, jamais une collision fausse).
    path: Vec<u16>,
    /// Items + spans de la peinture précédente.
    inc: Option<IncCx<'a>>,
    /// `prev.tree_epoch == epoch` — l'arbre est le même objet
    /// : le fast-path `bx_ptr` est valide (sinon une adresse
    /// recyclée pourrait mentir).
    same_tree: bool,
    /// Nœud dont le fond a été propagé au canvas (§14.2 —
    /// racine, ou `body` quand la racine n'a pas de fond) :
    /// sa propre boîte ne repeint pas son fond.
    canvas_bg_src: Option<explorer_dom::NodeId>,
    /// Compteur d'émissions par chemin (§B-26) — un porteur
    /// reporté (inline/positionné/fixe) doit être peint une
    /// fois par entrée de file ; au-delà de
    /// [`MAX_EMIT_PER_PATH`], la file s'auto-alimente (cycle
    /// de report) et on casse la boucle plutôt que de figer
    /// le processus.
    emit_per_path: std::collections::HashMap<Box<[u16]>, u32>,
    /// Décalages sticky `(dx, dy)` des enfants reportés —
    /// indexés par pointeur de boîte : le parent connaît son
    /// `bound` à l'enfilement (R-134 — un sticky peint en
    /// phase positionnée, plus en flux, doit garder sa
    /// translation d'épingle quel que soit le drain final).
    sticky_deltas: std::collections::HashMap<usize, (f32, f32)>,
    /// R-272 — transform `offset-*` des enfants à chemin
    /// (`offset-path`), indexée par pointeur : le bloc
    /// conteneur n'est connu qu'au niveau du parent — le
    /// calcul y est fait à l'émission de l'enfant, consommé
    /// dans `paint_box` au site `box_transform` (même patron
    /// que `sticky_deltas`).
    offset_tf: std::collections::HashMap<usize, explorer_layout::transform::Affine>,
    /// Total d'émissions reportées de la passe (§B-26) —
    /// plafond dur [`MAX_EMITS_TOTAL`] couvrant les cycles à
    /// chemins divergents que le cap par chemin ne voit pas.
    emits_total: u32,
    /// Émissions sauté par la borne §B-26 + premier chemin
    /// fautif — remontés dans [`DisplayList::dropped_emits`].
    dropped: u32,
    /// Premier chemin cyclé (§B-26), pour diagnostic.
    first_cycle_path: Option<Vec<u16>>,
    /// Collecteur de répartition §B-118 — `Some` uniquement
    /// sur les variantes `*_stats` (comptage opt-in).
    stats: Option<&'a mut EmitStats>,
    /// R-293 — sondes d'attribution de l'émission (audit
    /// perf archétypes) : µs dans `copy_span`+`repush_
    /// escaped`, µs dans le bookkeeping `escaped_paths`+
    /// `record_span`, compteurs — spans réutilisés,
    /// peintures fraîches, sous-arbres culés `may_escape`.
    /// Rapportés en `emit.parts` une fois par passe ; le
    /// reste du temps d'emit = traversée + émission réelle.
    us_copy: u64,
    us_book: u64,
    n_reuse: u32,
    n_paint: u32,
    n_culled: u32,
    /// Pile des clips `overflow` actifs — poussée au
    /// `PushClip` de `paint_box_impl`, retirée au `PopClip`
    /// (R-212) : les porteurs reportés (positionnés, inlines,
    /// fixed) en capturent un instantané à l'enfilement —
    /// le drain lointain les ré-émet autour de la boîte, sinon
    /// un `overflow:hidden` ancêtre perd ses descendants
    /// différés (nav de mesure vimeo visible, `h-0` outrepassé).
    clip_stack: Vec<(Rect, [f32; 4])>,
    /// Longueur de `clip_stack` à l'entrée du plus proche
    /// ancêtre positionné (son propre clip compris) — borne
    /// supérieure des clips applicables à un `absolute`
    /// différé : seuls les clips de son bloc conteneur et de
    /// ses ancêtres le rognent, les clippers entre le CB et
    /// lui ne s'appliquent pas (§11.1.1).
    pos_floor: usize,
    /// Clips `overflow` applicables par chemin reporté —
    /// instantané pris à l'enfilement, consommé au drain
    /// (`emit_scoped_at`). Le chemin est la clé unique du
    /// report ; les entrées non drainées meurent avec la passe.
    defer_clips: DeferClips,
}

/// Source de ré-usage incrémental.
#[derive(Clone, Copy)]
struct IncCx<'a> {
    /// Items de la liste précédente (copiés par plage).
    items: &'a [DisplayItem],
    /// Spans de la liste précédente.
    spans: &'a SpanMap,
}

impl PaintCx<'_> {
    /// Applique `f` au collecteur de répartition s'il existe
    /// (variantes `*_stats` — sinon no-op, zéro coût).
    fn stat(&mut self, f: impl FnOnce(&mut EmitStats)) {
        if let Some(s) = self.stats.as_mut() {
            f(s);
        }
    }

    /// Enregistre la plage `[start, end)` émise pour `bx`
    /// sous le chemin courant — flags de ré-usage calculés.
    /// `escaped` = porteurs reportés au `defer` hérité
    /// (chemins **relatifs** à `bx` — voir `SpanRec::escaped`).
    fn record_span(
        &mut self,
        bx: &LayoutBox,
        start: u32,
        end: u32,
        off: (f32, f32),
        sig: u64,
        escaped: Box<[EscapedDefer]>,
    ) {
        let mut flags = 0u8;
        if bx.position == Position::Fixed {
            flags |= F_FIXED;
        }
        if is_media(bx, self.images) {
            flags |= F_NO_REUSE;
        }
        if bx.children.iter().any(|c| c.position == Position::Sticky) {
            flags |= F_STICKY_KID;
        }
        self.spans.insert(
            self.path.clone().into_boxed_slice(),
            SpanRec {
                start,
                end,
                rect: bx.rect,
                off,
                sig,
                flags,
                bx_ptr: std::ptr::from_ref(bx) as usize,
                lines_ptr: bx.lines.as_ptr() as usize,
                kids_ptr: if bx.children.is_empty() {
                    usize::MAX
                } else {
                    bx.children.as_ptr() as usize
                },
                kids_len: u32::try_from(bx.children.len()).unwrap_or(u32::MAX),
                escaped,
            },
        );
    }
}

/// Extrait les chemins des porteurs poussés dans `defer`
/// depuis l'indice `base`, relativisés à `path` (le chemin
/// de la boîte en cours d'enregistrement).
fn escaped_paths(
    defer: &[(Vec<u16>, &LayoutBox)],
    base: usize,
    path: &[u16],
    cx: &PaintCx,
) -> Box<[EscapedDefer]> {
    defer[base..]
        .iter()
        .filter_map(|(p, _)| {
            p.strip_prefix(path).map(|rel| {
                (
                    Box::from(rel),
                    cx.defer_clips
                        .get(p.as_slice())
                        .cloned()
                        .unwrap_or_default(),
                )
            })
        })
        .collect()
}

/// Résout un chemin relatif depuis `bx` vers la boîte cible.
fn resolve_rel<'a>(bx: &'a LayoutBox, rel: &[u16]) -> Option<&'a LayoutBox> {
    let mut cur = bx;
    for &i in rel {
        cur = cur.children.get(usize::from(i))?;
    }
    Some(cur)
}

/// Sans images (tout `<img>` tombe en boîte `alt`) — voir
/// [`build_display_list_imaged`].
#[must_use]
pub fn build_display_list(
    root: &LayoutBox,
    viewport_w: f32,
    viewport_h: f32,
    scroll_y: f32,
) -> DisplayList {
    build_display_list_imaged(
        root,
        viewport_w,
        viewport_h,
        scroll_y,
        &crate::ImageTable::new(),
    )
}

/// `rect` translaté de `dy`.
fn shifted(r: Rect, off: (f32, f32)) -> Rect {
    Rect {
        x: r.x + off.0,
        y: r.y + off.1,
        ..r
    }
}

/// Style usé d'un côté de bordure.
fn border_style(v: Option<&str>) -> BorderStyle {
    match v {
        Some("solid") => BorderStyle::Solid,
        Some("dotted" | "dashed") => BorderStyle::Dotted,
        Some("double") => BorderStyle::Double,
        Some("groove") => BorderStyle::Groove,
        Some("ridge") => BorderStyle::Ridge,
        Some("inset") => BorderStyle::Inset,
        Some("outset") => BorderStyle::Outset,
        _ => BorderStyle::None,
    }
}

/// `prop` → px pour le rendu : unités absolues seulement
/// (`px`, `pt`, `cm`…) ; les mots-clés d'épaisseur CSS
/// (`thin`/`medium`/`thick`) valent 1/3/5 px ; `em`/`rem` et
/// pourcents sont hors portée ici (pas de contexte de fonte)
/// → repli `fallback`.
/// `border-<coin>-radius` → px (R-167) : `%` du plus petit
/// côté de la boîte (approximation — la spec résout x% sur
/// la largeur, y% sur la hauteur pour les rayons elliptiques ;
/// ici rayon circulaire par coin), unité absolue sinon, 0 si
/// absent/invalide.
#[allow(clippy::cast_possible_truncation)]
fn radius_px(raw: Option<&str>, basis: f32) -> f32 {
    let Some(v) = raw.map(str::trim) else {
        return 0.0;
    };
    if let Some(p) = v.strip_suffix('%') {
        return p.trim().parse::<f32>().unwrap_or(0.0) / 100.0 * basis;
    }
    parse_length(v)
        .and_then(|l| l.unit.px_factor().map(|f| l.value * f))
        .unwrap_or(0.0) as f32
}

/// Rayons des 4 coins `[tl, tr, br, bl]` en px, bornés à la
/// moitié du plus petit côté (§ couvercle — deux rayons
/// adjacents ne se chevauchent pas, approximation).
fn corner_radii(st: &explorer_css::ComputedStyle, rect: &Rect) -> [f32; 4] {
    let basis = rect.width.min(rect.height).max(0.0);
    let cap = basis / 2.0;
    [
        radius_px(st.get("border-top-left-radius"), basis).min(cap),
        radius_px(st.get("border-top-right-radius"), basis).min(cap),
        radius_px(st.get("border-bottom-right-radius"), basis).min(cap),
        radius_px(st.get("border-bottom-left-radius"), basis).min(cap),
    ]
}

/// `corner-*-shape` (css-borders-4 §3.7 — R-270) →
/// exposants `K` de la superellipse par coin \[tl,tr,br,bl]
/// : `round`=1 (e=2), `squircle`=2 (e=4), `square`=+∞,
/// `bevel`=0 (e=1), `scoop`=-1 (e=0.5, concave),
/// `notch`=-∞ ; `superellipse(k)`/`±infinity` = k/±∞.
/// `round` absent → 1.0 partout (comportement historique).
/// `PushClipRounded`/`BoxShadow`/`outline` restent ronds —
/// borne documentée : la forme ne suit pas encore ces
/// surfaces.
fn corner_ks(st: &explorer_css::ComputedStyle) -> [f32; 4] {
    fn k_of(v: &str) -> Option<f32> {
        Some(match v.trim() {
            "round" => 1.0,
            "squircle" => 2.0,
            "square" => f32::INFINITY,
            "bevel" => 0.0,
            "scoop" => -1.0,
            "notch" => f32::NEG_INFINITY,
            other => {
                let inner = other
                    .strip_prefix("superellipse(")
                    .and_then(|s| s.strip_suffix(')'))?
                    .trim();
                match inner {
                    "infinity" => f32::INFINITY,
                    "-infinity" => f32::NEG_INFINITY,
                    _ => inner.parse::<f32>().ok()?,
                }
            }
        })
    }
    [
        st.get("corner-top-left-shape"),
        st.get("corner-top-right-shape"),
        st.get("corner-bottom-right-shape"),
        st.get("corner-bottom-left-shape"),
    ]
    .map(|v| v.and_then(k_of).unwrap_or(1.0))
}

/// Une couche `box-shadow` : `inset? x y [blur [spread]]
/// [color]` dans n'importe quel ordre (spec : 2 à 4
/// longueurs consécutives + couleur optionnelle).
#[allow(clippy::cast_possible_truncation, clippy::type_complexity)]
fn parse_box_shadow_layer(
    st: &explorer_css::ComputedStyle,
    raw: &str,
) -> Option<((f32, f32), f32, f32, Color, bool)> {
    let mut lens = Vec::with_capacity(4);
    let mut color = None;
    let mut inset = false;
    for tok in raw.split_whitespace() {
        if tok.eq_ignore_ascii_case("inset") {
            inset = true;
        } else if let Some(l) = parse_length(tok) {
            lens.push(l.unit.px_factor().map_or(0.0, |f| l.value * f) as f32);
        } else if let Some(c) = token_color(st, tok) {
            color = Some(c);
        }
        // Fonctions couleur (`rgb(…)`, `var(…)`) déjà
        // résolues en cascade quand supportées.
    }
    if lens.len() < 2 {
        return None;
    }
    let blur = lens.get(2).copied().unwrap_or(0.0).max(0.0);
    let spread = lens.get(3).copied().unwrap_or(0.0);
    Some((
        (lens[0], lens[1]),
        blur,
        spread,
        // Défaut spec = `currentcolor` (R-265) — la `color`
        // calculée de la boîte, plus du noir fixe.
        color.or_else(|| st.color("color")).unwrap_or(Color::BLACK),
        inset,
    ))
}

/// `box-shadow` → un `DisplayItem::BoxShadow` par couche
/// (`none`/couches sans 2 longueurs = sautés ; `inset`
/// documenté non peint — ombres externes seulement).
fn emit_box_shadows(st: &explorer_css::ComputedStyle, rect: &Rect, items: &mut Vec<DisplayItem>) {
    let Some(v) = st.get("box-shadow") else {
        return;
    };
    if v.trim().eq_ignore_ascii_case("none") {
        return;
    }
    let radii = corner_radii(st, rect);
    for layer in bg_layers(v) {
        let Some((offset, blur, spread, color, inset)) = parse_box_shadow_layer(st, layer.trim())
        else {
            continue;
        };
        if inset {
            continue; // ombre interne non peinte (approx.).
        }
        items.push(DisplayItem::BoxShadow {
            rect: *rect,
            offset,
            blur,
            spread,
            color,
            radii,
        });
    }
}

#[allow(clippy::cast_possible_truncation)]
fn prop_px(st: &explorer_css::ComputedStyle, prop: &str, fallback: f32) -> f32 {
    let raw = st.get_or(prop, "");
    match raw {
        "thin" => 1.0,
        "medium" => 3.0,
        "thick" => 5.0,
        _ => parse_length(raw)
            .map(|l| match l.unit.px_factor() {
                Some(f) => l.value * f,
                // Unités font-relatives : la cascade stocke la
                // `font-size` calculée en px absolu (R-17) —
                // `1em` de bordure = 12 px sous `.picture`,
                // pas le repli `medium` (Acid2 — les bordures
                // 1em peinaient 3 px au lieu de 12).
                None => l.value * font_size_of(st) * em_factor(l.unit),
            })
            .map_or(fallback, |v| v as f32),
    }
}

/// La valeur `background-image` contient une couche `url(…)`
/// — n'importe où : `linear-gradient(…),url(…)` (sprites
/// Wikipedia, R-71) est un `background-image` valide dont la
/// couche image est en seconde position. Insensible à la
/// casse, comme l'extracteur `url(…)` de la collecte.
pub(crate) fn bg_has_url(v: &str) -> bool {
    v.as_bytes()
        .windows(4)
        .any(|w| w.eq_ignore_ascii_case(b"url("))
}

/// La valeur contient une fonction dégradé (`linear-gradient(`
/// et variantes — repérées par leur suffixe `gradient(`,
/// insensible à la casse). Seul le dégradé **linéaire** se
/// peint aujourd'hui ; `radial`/`conic` sont acceptés au parse
/// (déclaration préservée) mais non rasterisés.
fn bg_has_gradient(v: &str) -> bool {
    v.as_bytes()
        .windows(9)
        .any(|w| w.eq_ignore_ascii_case(b"gradient("))
}

/// La valeur `background-image` porte une couche potentiellement
/// peignable (`url(…)` ou dégradé) — issue #10.
fn bg_has_image(v: &str) -> bool {
    bg_has_url(v) || bg_has_gradient(v)
}

/// Couches d'une valeur `background-image` — éclatées sur les
/// virgules hors parenthèses (`linear-gradient(…),url(…)`).
pub(crate) fn bg_layers(v: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0u32;
    let mut start = 0usize;
    for (i, ch) in v.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&v[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&v[start..]);
    out
}

/// Émet les couches `background-image` de la boîte, **de la
/// dernière déclarée (fond) vers la première (dessus)** —
/// ordre de peinture des couches multiples CSS. Chaque couche
/// `url(…)` puise son bitmap décodé dans `images` ; chaque
/// couche `linear-gradient(…)` est rasterisée à la taille de
/// la zone de positionnement (un dégradé n'a pas de taille
/// intrinsèque — il couvre l'aire, §images-3). Une couche dont
/// la source manque (url non décodée, dégradé non supporté)
/// est simplement sautée, les autres peignent.
// tw/th ci-dessous sont vérifiés finis et > 0 juste avant le
// cast (`round().max(1.0)`) — ni troncature utile ni signe
// perdu, au pire un arrondi pixel.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
/// `clip-path` → clip géométrique (R-187) : `polygon` →
/// `PushClipPath` réel ; `inset`/`circle`/`ellipse` →
/// `PushClip`/`PushClipRounded` rectangulaire (approx.
/// documentée). `%` relatifs à la border box. Limites
/// documentées : les descendants `absolutely`/`fixed`
/// positionnés passent par `pos_defer`/`fixed_defer` et
/// échappent au clip de l'ancêtre ; `path()`/`url()` et la
/// `reference-box` explicite ne sont pas portés.
#[allow(clippy::many_single_char_names)]
fn clip_path_of(st: &explorer_css::ComputedStyle, rect: &Rect) -> Option<DisplayItem> {
    let v = st.get("clip-path")?.trim();
    if v == "none" {
        return None;
    }
    // `<length>`/`%` → px absolu (`%` contre `base`) ;
    // autre/invalide → `None` (jamais de clip faux).
    let len = |t: &str, base: f32| -> Option<f32> {
        let t = t.trim();
        if let Some(s) = t.strip_suffix('%') {
            return s.trim().parse::<f32>().ok().map(|p| p * base / 100.0);
        }
        parse_length(t).and_then(|l| l.unit.px_factor().map(|f| (l.value * f) as f32))
    };
    if let Some(args) = v.strip_prefix("polygon(").and_then(|v| v.strip_suffix(')')) {
        // `fill-rule` en tête (`evenodd`/`nonzero`) non lu
        // — `Winding` au raster.
        let points: Vec<(f32, f32)> = args
            .split(',')
            .filter_map(|pt| {
                let mut it = pt.split_whitespace();
                match (it.next(), it.next()) {
                    (Some(x), Some(y)) => {
                        Some((rect.x + len(x, rect.width)?, rect.y + len(y, rect.height)?))
                    }
                    _ => None,
                }
            })
            .collect();
        if points.len() >= 3 {
            return Some(DisplayItem::PushClipPath { points });
        }
        return None;
    }
    if let Some(args) = v.strip_prefix("inset(").and_then(|v| v.strip_suffix(')')) {
        let mut toks = args.split_whitespace();
        let vals: Vec<&str> = toks.by_ref().take_while(|p| *p != "round").collect();
        let (t, r, b, l) = match vals.as_slice() {
            [a] => (*a, *a, *a, *a),
            [a, b] => (*a, *b, *a, *b),
            [a, b, c] => (*a, *b, *c, *b),
            [a, b, c, d, ..] => (*a, *b, *c, *d),
            _ => return None,
        };
        // `top`/`bottom` contre la hauteur, `left`/`right`
        // contre la largeur — `px` et `%` exacts.
        let (top, right, bottom, left) = (
            len(t, rect.height)?,
            len(r, rect.width)?,
            len(b, rect.height)?,
            len(l, rect.width)?,
        );
        let clip = Rect {
            x: rect.x + left,
            y: rect.y + top,
            width: (rect.width - left - right).max(0.0),
            height: (rect.height - top - bottom).max(0.0),
        };
        // `round <r>` : premier rayon appliqué aux 4 coins
        // (coins différenciés/`/` elliptique hors MVP).
        let radius = toks.next().and_then(|t| len(t, clip.width)).unwrap_or(0.0);
        return if radius > 0.0 {
            Some(DisplayItem::PushClipRounded {
                rect: clip,
                radii: [radius; 4],
            })
        } else {
            Some(DisplayItem::PushClip(clip))
        };
    }
    for fn_name in ["circle", "ellipse"] {
        if let Some(args) = v
            .strip_prefix(fn_name)
            .and_then(|v| v.strip_prefix('('))
            .and_then(|v| v.strip_suffix(')'))
        {
            // `circle(r)`/`ellipse(rx ry)` — `%` contre l'axe
            // porté (approx. du `√(w²+h²)/√2` spec de
            // `circle`), longueurs exactes, absent/invalide →
            // demi-axe (≈ `closest-side`, biais « visible »).
            // `at <position>` non lu — centre boîte.
            let toks: Vec<&str> = args.split_whitespace().take_while(|t| *t != "at").collect();
            let half = |t: Option<&str>, base: f32| -> f32 {
                t.and_then(|t| len(t, base))
                    .map_or(base / 2.0, |v| v.max(0.0))
            };
            let rx = half(toks.first().copied(), rect.width);
            let ry = if fn_name == "ellipse" {
                half(toks.get(1).copied(), rect.height)
            } else {
                half(toks.first().copied(), rect.height)
            };
            let (cx, cy) = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
            return Some(DisplayItem::PushClip(Rect {
                x: cx - rx,
                y: cy - ry,
                width: rx * 2.0,
                height: ry * 2.0,
            }));
        }
    }
    None
}

/// R-184 — rects des fragments de ligne dans le sous-arbre
/// de `bx` (clip `background-clip:text` : les lignes vivent
/// sur les porteurs inline anonymes, pas sur la boîte).
/// Chaque run donne un rect de clip — approximation run-
/// par-run du clip glyphe-par-glyphe.
fn text_clip_rects(bx: &LayoutBox) -> Vec<Rect> {
    fn walk(b: &LayoutBox, out: &mut Vec<Rect>) {
        for line in &b.lines {
            for f in &line.fragments {
                out.push(f.rect);
            }
        }
        for c in &b.children {
            explorer_common::deep(|| walk(c, out));
        }
    }
    let mut out = Vec::new();
    walk(bx, &mut out);
    out
}

/// R-184 — `background-blend-mode` : tag compact porté
/// par `DisplayItem::BackgroundImage::blend` — 0 = `normal`
/// (repli valeur inconnue/absente), l'index+1 suit l'ordre
/// [`BLEND_MODES`]. Le raster traduit le tag en
/// `tiny_skia::BlendMode` — la display list reste sans
/// dépendance sur l'énum complète.
const BLEND_MODES: [&str; 15] = [
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color-dodge",
    "color-burn",
    "hard-light",
    "soft-light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
];

/// Keyword `background-blend-mode` → tag `BLEND_MODES`
/// (R-184). `normal`/inconnu → 0.
fn blend_tag(keyword: Option<&str>) -> u8 {
    match keyword {
        None | Some("normal") => 0,
        Some(k) => BLEND_MODES
            .iter()
            .position(|m| *m == k)
            .map_or(0, |i| u8::try_from(i + 1).unwrap_or(0)),
    }
}

#[allow(
    clippy::too_many_lines,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn emit_bg_layers(
    items: &mut Vec<DisplayItem>,
    bx: &LayoutBox,
    images: &crate::PaintImages,
    rect: Rect,
    area: (f32, f32),
    base: (f32, f32),
    off: (f32, f32),
) {
    let style = &bx.style;
    let node = bx.node;
    let Some(v) = style.get("background-image") else {
        return;
    };
    if v.is_empty() || v == "none" || !bg_has_image(v) {
        return;
    }
    // R-106/107 — par couche : `background-origin` (aire de
    // positionnement), `background-clip` (clip), `repeat`,
    // `position`, `size` sont des **listes** (séparées `, ` par
    // le shorthand multi-couches) — la i-ème valeur de chaque
    // longhand s'applique à la i-ème couche, la liste courte
    // se répète (css-backgrounds-3 §3.10). Les trois boîtes se
    // déduisent de `rect` (bordure), `area`/`base` (padding)
    // et `content_rect` (contenu — publique R-99).
    let content = content_rect(bx);
    let padding = Rect {
        x: base.0,
        y: base.1,
        width: area.0,
        height: area.1,
    };
    // i-ème valeur d'une longhand liste (ou repli liste vide /
    // hors de portée → `None`, le `match` du site retombe sur
    // le défaut CSS).
    let layer_value = |name: &str, i: usize| -> Option<String> {
        let raw = style.get(name)?;
        let parts = bg_layers(raw);
        parts
            .get(i)
            .or_else(|| parts.last())
            .map(|s| (*s).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let fs = font_size_of(style);
    let layers = bg_layers(v);
    // La table `backgrounds` est clé `NodeId → image` et la
    // collecte ne fetch que le **premier** `url()` du style
    // calculé : l'image stockée appartient à la première
    // couche url — la donner aux couches suivantes
    // repeindrait cette même bitmap avec leurs paramètres
    // (mesuré : w3.org/TR/REC-html32 — le filigrane
    // `no-repeat` était retilé sur tout le viewport par la
    // couche texture `repeat`). Les autres couches url sont
    // sautées tant que la session ne fetch pas leurs urls.
    let first_url = layers.iter().position(|l| bg_has_url(l));
    for (i, layer) in layers.iter().enumerate().rev() {
        let clip_text = layer_value("background-clip", i).as_deref() == Some("text");
        let clip = match layer_value("background-clip", i).as_deref() {
            Some("content-box") => content,
            Some("padding-box") => padding,
            // `text` (R-184 — clip par fragment ci-dessous),
            // `border-box` (défaut), `border-area` — repli bordure.
            _ => rect,
        };
        let (l_area, l_base) = match layer_value("background-origin", i).as_deref() {
            Some("border-box") => ((rect.width, rect.height), (rect.x, rect.y)),
            Some("content-box") => ((content.width, content.height), (content.x, content.y)),
            // `padding-box` = défaut — identique à l'aire reçue.
            _ => (area, base),
        };
        // `space`/`round` (css-backgrounds-3 §3.5) : modes
        // propres (gap distribué / tuile étirée) — pas
        // l'approximation `repeat` des tuiles booléennes.
        let mode = |t: &str| match t {
            "no-repeat" => 1u8,
            "space" => 2,
            "round" => 3,
            _ => 0,
        };
        let repeat = {
            let v = layer_value("background-repeat", i).unwrap_or("repeat".to_string());
            match v.trim() {
                "repeat-x" => (0u8, 1u8),
                "repeat-y" => (1u8, 0u8),
                _ => {
                    let mut it = v.split_whitespace();
                    let a = it.next().map_or(0, mode);
                    let b = it.next().map_or(a, mode);
                    (a, b)
                }
            }
        };
        let img = if bg_has_url(layer) {
            let from_layers = node.and_then(|n| {
                images
                    .background_layers
                    .get(&n)
                    .and_then(|v| v.iter().find(|(idx, _)| usize::from(*idx) == i))
                    .map(|(_, img)| img.clone())
            });
            from_layers.or_else(|| {
                // Repli contrat historique : `backgrounds[n]`
                // sert la première couche url (les fetchers qui
                // ne remplissent pas `background_layers`
                // conservent leur comportement, R-203).
                if Some(i) == first_url {
                    node.and_then(|n| images.backgrounds.get(&n)).cloned()
                } else {
                    None
                }
            })
        } else if bg_has_gradient(layer) {
            crate::gradient::decode_gradient(layer, l_area.0, l_area.1, fs).map(std::sync::Arc::new)
        } else {
            None
        };
        if let Some(img) = img {
            // R-95 — `background-size` dimensionne la tuile
            // (naturelle par défaut) ; sans lui un bitmap plus
            // grand que la boîte n'en peignait que le coin —
            // mesuré : `.votearrow` de news.ycombinator.com,
            // SVG 32×32 à motif centré dans une boîte 10×10.
            // R-107 : la taille est lue **par couche**.
            let (tw, th) = bg_size(
                layer_value("background-size", i).as_deref(),
                l_area,
                (img.width as f32, img.height as f32),
                fs,
            );
            if !(tw.is_finite() && th.is_finite()) || tw <= 0.0 || th <= 0.0 {
                continue; // `background-size:0` ou non résolu.
            }
            let (tw, th) = (tw.round().max(1.0), th.round().max(1.0));
            let img = if tw * th > MAX_TILE_PX as f32 {
                // Borne : tuile naturelle plutôt qu'une
                // allocation hostile (`cover` sur une très
                // grande aire, `background-size` énorme).
                img
            } else if tw as u32 != img.width || th as u32 != img.height {
                std::sync::Arc::new(scale_tile(&img, tw as u32, th as u32))
            } else {
                img
            };
            // `%` de `background-position` mesure
            // `(zone − image)` : un dégradé calibré à la zone
            // rend donc les positions relatives neutres, les
            // offsets absolus (`0 -667px` sprites) actifs —
            // conforme §14.2.1. La dimension retenue est la
            // tuile **mise à l'échelle** (spec §images-3).
            let (mut px, mut py) = bg_position(
                layer_value("background-position", i).as_deref(),
                l_area,
                (img.width as f32, img.height as f32),
                fs,
            );
            // R-261 — longhands `background-position-x`/`-y`
            // (css-backgrounds-4 §3.5) : chaque axe déclaré
            // écrase l'axe correspondant du shorthand (la
            // cascade les stocke verbatim, ce sont des
            // longhands indépendants — `background`/
            // `background-position` continuent de passer par
            // le nom combiné).
            if let Some(x) = bg_axis_position(
                layer_value("background-position-x", i).as_deref(),
                l_area.0,
                img.width as f32,
                fs,
            ) {
                px = x;
            }
            if let Some(y) = bg_axis_position(
                layer_value("background-position-y", i).as_deref(),
                l_area.1,
                img.height as f32,
                fs,
            ) {
                py = y;
            }
            let blend = blend_tag(layer_value("background-blend-mode", i).as_deref());
            // R-184 — `background-clip:text` : la couche est
            // émise une fois par fragment de ligne (clip
            // propre à chaque run — `PushClip` existant,
            // aucune primitive de masque au MVP).
            if clip_text {
                for fr in text_clip_rects(bx) {
                    {
                        // `fr` est en coords layout —
                        // `off` aligne le clip sur la
                        // position peinte (rejeu décalé).
                        let f = shifted(fr, off);
                        items.push(DisplayItem::PushClip(f));
                        items.push(DisplayItem::BackgroundImage {
                            rect: clip,
                            img: img.clone(),
                            repeat,
                            origin: (l_base.0 + px, l_base.1 + py),
                            blend,
                        });
                        items.push(DisplayItem::PopClip);
                    }
                }
            } else {
                items.push(DisplayItem::BackgroundImage {
                    rect: clip,
                    img,
                    repeat,
                    origin: (l_base.0 + px, l_base.1 + py),
                    blend,
                });
            }
        }
    }
}

/// `text-emphasis` → (kind, couleur, over) ou `None`
/// (R-186) : `dot|circle|double-circle|triangle|sesame`,
/// `filled`/`open`, couleur optionnelle — `none` → `None`,
/// position `under` → `over=false`.
fn emphasis_kind(st: &explorer_css::ComputedStyle, ink: Color) -> Option<(u8, Color, bool)> {
    let raw = st
        .get("text-emphasis-style")
        .or_else(|| st.get("text-emphasis"))?;
    if raw.trim() == "none" {
        return None;
    }
    let mut kind = 0u8;
    let mut found = false;
    let mut color = st.color("text-emphasis-color").unwrap_or(ink);
    let over = st
        .get("text-emphasis-position")
        .is_none_or(|v| v.trim() != "under");
    for tok in raw.split_whitespace() {
        match tok {
            "dot" => {
                kind = 0;
                found = true;
            }
            "circle" => {
                kind = 1;
                found = true;
            }
            "double-circle" => {
                kind = 2;
                found = true;
            }
            "triangle" => {
                kind = 3;
                found = true;
            }
            "sesame" => {
                kind = 4;
                found = true;
            }
            "open" => kind |= 8,
            "filled" => {}
            _ => {
                if let Some(c) = token_color(st, tok) {
                    color = c;
                }
            }
        }
    }
    // `<string>` custom (caractère unique) : non porté au
    // MVP — repli `dot` documenté.
    if !found {
        return Some((0, color, over));
    }
    Some((kind, color, over))
}

/// R-M4 — émission des fantômes `text-shadow` pour un
/// fragment : une run par couche sous le texte ; `blur`
/// ≈ 4 fantômes croisés ±blur/2 à alpha/3 (documenté,
/// pas de gaussien).
#[allow(clippy::too_many_arguments)]
fn emit_text_shadows(
    st: &explorer_css::ComputedStyle,
    x: f32,
    base_y: f32,
    run: &std::sync::Arc<ShapedRun>,
    fs: f32,
    ink: Color,
    items: &mut Vec<DisplayItem>,
) {
    for (soff, sblur, scolor) in text_shadows(st, ink) {
        let mut ghost = |dx: f32, dy: f32, a: u8| {
            let mut c = scolor;
            c.a = a;
            items.push(DisplayItem::Text {
                origin: (x + soff.0 + dx, base_y + soff.1 + dy),
                run: run.clone(),
                font: font_request(st),
                color: c,
                size_px: fs,
            });
        };
        if sblur > 0.0 {
            ghost(sblur / 2.0, 0.0, scolor.a / 3);
            ghost(-sblur / 2.0, 0.0, scolor.a / 3);
            ghost(0.0, sblur / 2.0, scolor.a / 3);
            ghost(0.0, -sblur / 2.0, scolor.a / 3);
        } else {
            ghost(0.0, 0.0, scolor.a);
        }
    }
}

/// `text-shadow` → couches `(offset, blur, color)`
/// (R-M4) : `x y [blur] [color]` par virgule de niveau
/// 0 ; `none` → vide. Le flou est rendu en fantômes
/// croisés à l'émission (approx. documentée).
#[allow(clippy::cast_possible_truncation)]
fn text_shadows(st: &explorer_css::ComputedStyle, default: Color) -> Vec<((f32, f32), f32, Color)> {
    let Some(v) = st.get("text-shadow") else {
        return Vec::new();
    };
    if v.trim().eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    bg_layers(v)
        .iter()
        .filter_map(|layer| {
            let mut lens = Vec::with_capacity(3);
            let mut color = None;
            for tok in layer.split_whitespace() {
                if let Some(l) = parse_length(tok) {
                    lens.push(l.unit.px_factor().map_or(0.0, |f| l.value * f) as f32);
                } else if let Some(c) = token_color(st, tok) {
                    color = Some(c);
                }
            }
            if lens.len() < 2 {
                return None;
            }
            Some((
                (lens[0], lens[1]),
                lens.get(2).copied().unwrap_or(0.0).max(0.0),
                color.unwrap_or(default),
            ))
        })
        .collect()
}

/// Marqueur de `display: list-item` (issue #9) : la puce se
/// peint dans la gouttière avant la content box
/// (`list-style-position: outside` — l'espace est laissé par
/// `ul { padding-left: 40px }`), alignée verticalement sur la
/// ligne de base de la première ligne de contenu (spec
/// §marker). Émise après le fond de la boîte et avant ses
/// lignes → sous le texte. `inside` dessine au début du
/// contenu (le texte n'est pas décalé — résidu).
/// `filter` → ops (R-M5) : tokens `fn(arg)` de la
/// valeur ; `none`/aucune op connue → vide (pas de
/// bracket émis). `drop-shadow()`/`url()` sautés.
#[allow(clippy::cast_possible_truncation)]
fn parse_filter(st: &explorer_css::ComputedStyle) -> Vec<FilterOp> {
    let Some(v) = st.get("filter") else {
        return Vec::new();
    };
    if v.trim().eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    // `1.2` → 1.2 ; `120%` → 1.2 (facteurs) ; `50%`/`0.5`
    // → 0.5 (parts 0..1) — mêmes tables pour les deux.
    let num_or_pct = |a: &str| -> Option<f32> {
        let t = a.trim();
        t.strip_suffix('%')
            .and_then(|p| p.parse::<f32>().ok())
            .map(|v| v / 100.0)
            .or_else(|| t.parse::<f32>().ok())
    };
    let mut out = Vec::new();
    let mut rest = v;
    while let Some(p) = rest.find('(') {
        let name = rest[..p].trim().to_ascii_lowercase();
        let inner = &rest[p + 1..];
        //  de fermeture de LA fonction — scan paren-
        // équilibré :  a des parens
        // imbriquées, la première  appartient au color.
        let mut depth = 0i32;
        let Some(q) = inner.char_indices().find_map(|(i, c)| match c {
            '(' => {
                depth += 1;
                None
            }
            ')' => {
                if depth == 0 {
                    Some(i)
                } else {
                    depth -= 1;
                    None
                }
            }
            _ => None,
        }) else {
            break;
        };
        let arg = inner[..q].trim();
        let op = match name.as_str() {
            "blur" => parse_length(arg)
                .and_then(|l| l.unit.px_factor().map(|f| (l.value * f) as f32))
                .map(|r| FilterOp::Blur(r.max(0.0))),
            "brightness" => num_or_pct(arg).map(FilterOp::Brightness),
            "contrast" => num_or_pct(arg).map(FilterOp::Contrast),
            "grayscale" => num_or_pct(arg).map(FilterOp::Grayscale),
            "invert" => num_or_pct(arg).map(FilterOp::Invert),
            "opacity" => num_or_pct(arg).map(FilterOp::Opacity),
            "saturate" => num_or_pct(arg).map(FilterOp::Saturate),
            "sepia" => num_or_pct(arg).map(FilterOp::Sepia),
            "hue-rotate" => arg
                .strip_suffix("deg")
                .and_then(|d| d.trim().parse::<f32>().ok())
                .map(FilterOp::HueRotate),
            "drop-shadow" => parse_drop_shadow(st, arg),
            // `url()` : hors scope.
            _ => None,
        };
        if let Some(o) = op {
            out.push(o);
        }
        rest = &inner[q + 1..];
    }
    out
}

/// `drop-shadow(<dx> <dy> [<blur>] [<color>])` — les
/// longueurs en tête (2 ou 3), la couleur optionnelle à la
/// fin (`parse_color` — accepte aussi les formes
/// `rgb(`…`)`/`rgba(`…`)` multi-tokens : tout ce qui n'est
/// pas une longueur à la fin est passé au parser couleur).
/// Défauts spec : blur 0, couleur `currentcolor` →
/// approximée à l'encre du déclarant (noir si absente).
#[allow(clippy::cast_possible_truncation)]
fn parse_drop_shadow(st: &explorer_css::ComputedStyle, arg: &str) -> Option<FilterOp> {
    let mut lens = Vec::new();
    let mut color = None;
    for tok in arg.split_whitespace() {
        if let Some(l) = parse_length(tok).and_then(|l| l.unit.px_factor().map(|f| l.value * f)) {
            lens.push(l as f32);
        } else {
            // Tout le reste (le ou les tokens couleur,
            // éventuellement collés par l'espace dans
            // `rgb(0, 0, 0)`) est recollé pour le parser
            // couleur (`currentcolor` inclus — R-265).
            let tail = arg[arg.find(tok).unwrap_or(0)..].trim();
            color = token_color(st, tail);
            break;
        }
    }
    if lens.len() < 2 {
        return None;
    }
    Some(FilterOp::DropShadow {
        dx: lens[0],
        dy: lens[1],
        blur: lens.get(2).copied().unwrap_or(0.0).max(0.0),
        // Défaut spec = `currentcolor` — la `color` calculée
        // du déclarant (R-265), plus du noir fixe.
        color: color.or_else(|| st.color("color")).unwrap_or(Color::BLACK),
    })
}

/// R-168 — `list-style-image` : la bitmap est fetchée
/// comme les `url()` CSS (clé `NodeId` → `backgrounds`)
/// et peinte dans la gouttière à la place du marqueur —
/// absent/`none` → `false`, repli `list-style-type`
/// (spec).
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn emit_image_marker(
    bx: &LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    images: &crate::PaintImages,
) -> bool {
    if !bx
        .style
        .get("list-style-image")
        .is_some_and(|v| v.contains("url("))
    {
        return false;
    }
    let Some(img) = bx.node.and_then(|n| images.backgrounds.get(&n)) else {
        return false;
    };
    let fs = font_size_of(&bx.style) as f32;
    let iw = img.width as f32;
    let ih = img.height as f32;
    let baseline_y = bx
        .lines
        .first()
        .map_or_else(|| bx.rect.y + fs, |l| l.rect.y + l.baseline);
    let cy = baseline_y - fs * 0.3;
    let content_x = content_rect(bx).x;
    let inside = bx
        .style
        .get("list-style-position")
        .is_some_and(|p| p.trim() == "inside");
    let cx = if inside {
        content_x + iw / 2.0
    } else {
        content_x - fs * 0.34 - iw / 2.0
    };
    items.push(DisplayItem::Image {
        rect: Rect {
            x: cx - iw / 2.0 + off.0,
            y: cy - ih / 2.0 + off.1,
            width: iw,
            height: ih,
        },
        img: std::sync::Arc::clone(img),
    });
    true
}

/// R-181 — `column-rule` (css-multicol §5) : trait vertical
/// centré dans chaque `column-gap`, peint au-dessus du fond
/// du conteneur. La géométrie des colonnes est recomptée
/// depuis le style (formule de `flow_columns` — `%`/`px` via
/// `prop_px` ; `em`/`ch` approximés à la fonte — limite
/// documentée). `column-rule-style` ∈ solid|dotted|dashed|
/// double (groove/ridge/inset/outset → solid) ; `none`/
/// `hidden` ou absent → rien. `column-rule-width` :
/// thin|medium|thick = 1|3|5 px, sinon longueur.
#[allow(clippy::cast_possible_truncation)]
#[allow(clippy::cast_sign_loss)]
#[allow(clippy::cast_precision_loss)]
fn emit_column_rules(bx: &LayoutBox, items: &mut Vec<DisplayItem>, off: (f32, f32)) {
    let st = &bx.style;
    let Some(style_t) = st.get("column-rule-style") else {
        return;
    };
    let style_t = style_t.trim();
    if matches!(style_t, "none" | "hidden") {
        return;
    }
    let deco_style = match style_t {
        "dotted" => explorer_layout::inline::DECO_DOTTED,
        "dashed" => explorer_layout::inline::DECO_DASHED,
        "double" => explorer_layout::inline::DECO_DOUBLE,
        _ => explorer_layout::inline::DECO_SOLID,
    };
    let rw = match st.get("column-rule-width").map(str::trim) {
        Some("thin") => 1.0,
        Some("thick") => 5.0,
        Some("medium") | None => 3.0,
        _ => prop_px(st, "column-rule-width", 3.0),
    };
    if rw <= 0.0 {
        return;
    }
    let cr = content_rect(bx);
    let fs = prop_px(st, "font-size", 16.0);
    let count = st
        .get("column-count")
        .filter(|v| v.trim() != "auto")
        .and_then(|v| v.trim().parse::<u32>().ok());
    let col_w_spec = st
        .get("column-width")
        .filter(|v| v.trim() != "auto")
        .map(|_| prop_px(st, "column-width", 0.0));
    if count.is_none() && col_w_spec.is_none() {
        return;
    }
    let gap = match st.get("column-gap").map(str::trim) {
        None | Some("normal") => fs,
        _ => prop_px(st, "column-gap", fs),
    };
    let n = match (count, col_w_spec) {
        (Some(c), Some(w)) if w > 0.0 => {
            c.min(((cr.width + gap) / (w + gap).max(1.0)).floor().max(1.0) as u32)
        }
        (Some(c), _) => c,
        (None, Some(w)) if w > 0.0 => {
            ((cr.width + gap) / (w + gap).max(1.0)).floor().max(1.0) as u32
        }
        _ => 1,
    };
    if n <= 1 {
        return;
    }
    let col_w = (cr.width - gap * (n as f32 - 1.0)) / n as f32;
    if col_w <= 0.0 {
        return;
    }
    let color = st
        .color("column-rule-color")
        .or_else(|| st.color("color"))
        .unwrap_or(Color::BLACK);
    // R-271 — `rule-break`/`rule-inset` (css-gaps-1 §4.2/4.4)
    // via `gap_rule_segments` ; `column-rule-inset-*` est le
    // longhand axe colonnes (expansion `rule-inset*`).
    let segs = gap_rule_segments(bx, cr.y, cr.y + cr.height);
    if segs.is_empty() {
        return;
    }
    for i in 1..n {
        let cx = cr.x + i as f32 * (col_w + gap) - gap * 0.5 - rw * 0.5;
        for (a, b) in &segs {
            items.push(DisplayItem::DecoRect {
                rect: Rect {
                    x: cx + off.0,
                    y: a + off.1,
                    width: rw,
                    height: b - a,
                },
                color,
                style: deco_style,
            });
        }
    }
}

/// R-271 — segments verticaux d'une règle de gap
/// (css-gaps-1 §4.2/4.4) : `column-rule-break`/`rule-break`
/// ∈ `none` peint à travers les spanners ; `spanning-item`/
/// `intersection` (et l'absence — css-multicol-1 §5.1 : les
/// règles ne traversent pas un `column-span:all`) coupent à
/// chaque spanner (rects des enfants `column-span:all` de
/// `bx`, posés pleine largeur). `column-rule-inset-start`/
/// `-end` raccourcissent les extrémités du trait.
fn gap_rule_segments(bx: &LayoutBox, y0: f32, y1: f32) -> Vec<(f32, f32)> {
    let st = &bx.style;
    let break_t = st
        .get("column-rule-break")
        .or_else(|| st.get("rule-break"))
        .map_or("spanning-item", str::trim);
    let break_spans = !matches!(break_t, "none");
    let y0 = y0 + prop_px(st, "column-rule-inset-start", 0.0);
    let y1 = y1 - prop_px(st, "column-rule-inset-end", 0.0);
    if y1 <= y0 {
        return Vec::new();
    }
    let mut segs = vec![(y0, y1)];
    if break_spans {
        let mut spans: Vec<(f32, f32)> = bx
            .children
            .iter()
            .filter(|c| {
                c.style
                    .get("column-span")
                    .is_some_and(|v| v.trim() == "all")
            })
            .map(|c| (c.rect.y, c.rect.y + c.rect.height))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (sy0, sy1) in spans {
            let mut next = Vec::with_capacity(segs.len() + 1);
            for (a, b) in segs {
                if sy1 <= a || sy0 >= b {
                    next.push((a, b));
                } else {
                    if sy0 > a {
                        next.push((a, sy0));
                    }
                    if sy1 < b {
                        next.push((sy1, b));
                    }
                }
            }
            segs = next;
        }
    }
    segs
}

fn emit_list_marker(
    bx: &LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    images: &crate::PaintImages,
) {
    // R-180 — `inside` : la puce est un `Text` injecté en
    // tête de ligne au layout — aucun marqueur externe à
    // peindre (il chevaucherait le texte décalé).
    if bx
        .style
        .get("list-style-position")
        .is_some_and(|p| p.trim() == "inside")
    {
        return;
    }
    if bx.kind != BoxKind::ListItem {
        return;
    }
    if emit_image_marker(bx, items, off, images) {
        return;
    }
    let kind = match bx.style.get("list-style-type").map(str::trim) {
        // Initiale spec = `disc` (absence ou `initial`).
        None | Some("disc" | "initial" | "inherit") => Some(MarkerKind::Disc),
        Some("circle") => Some(MarkerKind::Circle),
        Some("square") => Some(MarkerKind::Square),
        Some("none") => None,
        Some(other) => {
            // R-108 — marqueurs textuels (`decimal`, alpha,
            // roman…) : la display list ne shape pas, on émet
            // `MarkerText` — le raster résout la fonte et shape
            // le libellé au paint.
            emit_text_marker(bx, items, off, other);
            return;
        }
    };
    let Some(kind) = kind else { return };
    #[allow(clippy::cast_possible_truncation)]
    let fs = font_size_of(&bx.style) as f32;
    if !fs.is_finite() || fs <= 0.0 {
        return;
    }
    // Glyphe ≈ 1/3 em, gouttière ≈ 1/3 em avant le contenu —
    // les métriques des moteurs établis tournent autour.
    let d = (fs * 0.34).max(1.0);
    let gap = (fs * 0.34).max(1.0);
    let baseline_y = bx.lines.first().map_or_else(
        || {
            bx.rect.y
                + border_side_w(&bx.style, "top")
                + prop_px(&bx.style, "padding-top", 0.0)
                + fs
        },
        |l| l.rect.y + l.baseline,
    );
    let cy = baseline_y - fs * 0.3;
    let content_x = content_rect(bx).x;
    let inside = bx
        .style
        .get("list-style-position")
        .is_some_and(|p| p.trim() == "inside");
    let cx = if inside {
        content_x + d / 2.0
    } else {
        content_x - gap - d / 2.0
    };
    let color = bx.style.color("color").unwrap_or(Color::BLACK);
    items.push(DisplayItem::Marker {
        rect: Rect {
            x: cx - d / 2.0 + off.0,
            y: cy - d / 2.0 + off.1,
            width: d,
            height: d,
        },
        kind,
        color,
    });
}

#[allow(clippy::cast_precision_loss)]
/// Marqueur textuel `list-style-type` (R-108, css-lists §2) :
/// le libellé = compteur `marker_ordinal` rendu dans le type —
/// `decimal`/`decimal-leading-zero`, `lower|upper-alpha`
/// (=`latin`), `lower|upper-roman` ; type inconnu → `decimal`
/// (repli spec « unsupported counter style »). Ordinals ≤ 0
/// suivent la forme signée (`decimal` affiche `-1.`).
/// Position : aligné à droite dans la gouttière (spec marker
/// §3 — le marqueur finit avant le bord du contenu).
fn emit_text_marker(bx: &LayoutBox, items: &mut Vec<DisplayItem>, off: (f32, f32), ty: &str) {
    // R-177 — `::marker` : couleur/fonte du pseudo en
    // priorité (css-pseudo §4.2), repli style de l'élément.
    let mstyle = bx.marker_style.as_ref().unwrap_or(&bx.style);
    #[allow(clippy::cast_possible_truncation)]
    let fs = font_size_of(mstyle) as f32;
    if !fs.is_finite() || fs <= 0.0 {
        return;
    }
    // `<string>` (css-lists §3.3 — R-112) : le marqueur est
    // la chaîne **elle-même** (`list-style-type:"- "` →
    // `- `), pas un compteur — mesuré terrain :
    // `ul{list-style-type:"*"}` de l'IETF.
    let text = if let Some(s) = ty
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| ty.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
    {
        s.to_string()
    } else {
        explorer_layout::marker_label(ty, bx.marker_ordinal)
    };
    let baseline_y = bx.lines.first().map_or_else(
        || {
            bx.rect.y
                + border_side_w(&bx.style, "top")
                + prop_px(&bx.style, "padding-top", 0.0)
                + fs
        },
        |l| l.rect.y + l.baseline,
    );
    let content_x = content_rect(bx).x;
    let inside = bx
        .style
        .get("list-style-position")
        .is_some_and(|p| p.trim() == "inside");
    // Largeur approchée (~0.55em/car — chiffres et lettres
    // romaines) : la gouttière aligne le marqueur à droite ;
    // `inside` le pose au début du contenu.
    let est_w = text.chars().count() as f32 * fs * 0.55;
    let x = if inside {
        content_x
    } else {
        content_x - (fs * 0.4).max(1.0) - est_w
    };
    let color = mstyle.color("color").unwrap_or(Color::BLACK);
    items.push(DisplayItem::MarkerText {
        origin: (x + off.0, baseline_y + off.1),
        text,
        font: font_request(mstyle),
        color,
        size_px: fs,
    });
}

/// La boîte déclare un fond : `background-color` non
/// transparente ou `background-image` avec une couche
/// `url()`/dégradé (§14.2 — sélection de la source du fond de
/// canvas).
fn has_bg(st: &explorer_css::ComputedStyle) -> bool {
    st.color("background-color").is_some_and(|c| c.a > 0)
        || st.get("background-image").is_some_and(bg_has_image)
}

/// `font-size` calculée de la boîte en px (16 par défaut).
fn font_size_of(st: &explorer_css::ComputedStyle) -> f64 {
    let base = st
        .get("font-size")
        .and_then(parse_length)
        .and_then(|l| l.unit.px_factor().map(|f| l.value * f))
        .unwrap_or(16.0);
    // R-186 — `font-size-adjust` : la hauteur d'ex est
    // ramenée au facteur demandé — rapport appliqué vs
    // l'ex ≈ 0.5em de la fonte de repli (approximation
    // documentée, faces réelles ≈ 0.45-0.55em).
    st.get("font-size-adjust")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|a| *a > 0.0)
        .map_or(base, |a| base * (a / 0.5))
}

/// Rapport à l'em pour les unités font-relatives (`ex`/`ch`
/// ≈ 0.5em — approximation usuelle faute de métriques ici) ;
/// `rem`/`vw`/… retombent sur 1em faute de base racine dans
/// ce contexte.
fn em_factor(u: explorer_css::value::Unit) -> f64 {
    match u {
        explorer_css::value::Unit::Ex | explorer_css::value::Unit::Ch => 0.5,
        _ => 1.0,
    }
}

/// Couleur de bordure `side` (`border-top-color`…) — initiale
/// spec = **`currentcolor`** : une déclaration absente (ex.
/// `var()` non résolue retirée à la cascade) retombe sur la
/// `color` de la boîte, pas sur du noir (R-22 — les pastilles
/// `a.tag` de lobste.rs peignaient un cadre noir).
fn border_color_of(st: &explorer_css::ComputedStyle, side: &str) -> Color {
    st.color(side)
        .or_else(|| st.color("color"))
        .unwrap_or(Color::BLACK)
}

/// R-265 — couleur d'un **token** de valeur composée
/// (`box-shadow`, `text-shadow`, `text-emphasis`,
/// `drop-shadow()`) : `currentcolor` résolu sur le `color`
/// calculé de l'élément (css-color-3 §4.2) — un `parse_color`
/// brut retournait `None` et laissait le repli noir.
fn token_color(st: &explorer_css::ComputedStyle, tok: &str) -> Option<Color> {
    if tok.trim().eq_ignore_ascii_case("currentcolor") {
        return st.color("color").or(Some(Color::BLACK));
    }
    parse_color(tok)
}

/// Content box de la boîte = border box − bordures − padding
/// (les `em` sont résolus via `prop_px`/`font_size_of`).
///
/// Publique depuis R-99 : consommé hors crate par le composite
/// de sous-contextes `<iframe>` côté `explorer-core` (le
/// viewport enfant = content box de la boîte remplacée).
#[must_use]
pub fn content_rect(bx: &explorer_layout::LayoutBox) -> Rect {
    let st = &bx.style;
    let l = border_side_w(st, "left") + prop_px(st, "padding-left", 0.0);
    let t = border_side_w(st, "top") + prop_px(st, "padding-top", 0.0);
    let r = border_side_w(st, "right") + prop_px(st, "padding-right", 0.0);
    let b = border_side_w(st, "bottom") + prop_px(st, "padding-bottom", 0.0);
    Rect {
        x: bx.rect.x + l,
        y: bx.rect.y + t,
        width: (bx.rect.width - l - r).max(0.0),
        height: (bx.rect.height - t - b).max(0.0),
    }
}

/// Épaisseur de bordure `side` (`top`/`right`/…) — 0 si le
/// style du côté est absent ou `none` (`border: solid` sans
/// épaisseur peint `medium` = 3 px).
fn border_side_w(st: &explorer_css::ComputedStyle, side: &str) -> f32 {
    if st
        .get(&format!("border-{side}-style"))
        .is_none_or(|s| s.trim() == "none")
    {
        return 0.0;
    }
    prop_px(st, &format!("border-{side}-width"), 3.0)
}

/// `background-position` → décalage (px) de la tuile (0,0)
/// dans la zone de positionnement `area` (CSS2 §14.2.1 :
/// `%` mesure `(zone - image)` ; mots-clés = 0/50/100 % ;
/// une valeur seule → x, l'autre `center` ; les unités
/// font-relatives se résolvent sur `fs` — `0 -1em` de
/// `margin-collapse-020` décale le fond de sa propre
/// font-size). Acid2 utilise `1px 0` sur les yeux.
#[allow(clippy::cast_possible_truncation)]
fn bg_position(raw: Option<&str>, area: (f32, f32), img: (f32, f32), fs: f64) -> (f32, f32) {
    let toks: Vec<&str> = raw.unwrap_or("").split_whitespace().collect();
    if toks.is_empty() {
        return (0.0, 0.0);
    }
    let (x_tok, y_tok) = match toks.as_slice() {
        [a] => {
            // Seul un mot-clé vertical indique y.
            if matches!(*a, "top" | "bottom") {
                ("center", *a)
            } else {
                (*a, "center")
            }
        }
        [a, b, ..] => (*a, *b),
        _ => ("left", "top"),
    };
    (
        bg_pos_token(x_tok, area.0, img.0, fs),
        bg_pos_token(y_tok, area.1, img.1, fs),
    )
}

/// Une dimension de `background-position` : longueur/`%` =
/// offset depuis le coin, mot-clé → fraction de
/// (zone − image).
#[allow(clippy::cast_possible_truncation)]
fn bg_pos_token(t: &str, span: f32, dim: f32, fs: f64) -> f32 {
    match t {
        "left" | "top" => 0.0,
        "center" => (span - dim) / 2.0,
        "right" | "bottom" => span - dim,
        _ => t
            .strip_suffix('%')
            .and_then(|p| p.parse::<f32>().ok())
            .map_or_else(
                || {
                    parse_length(t)
                        .map(|l| match l.unit.px_factor() {
                            Some(f) => l.value * f,
                            None => l.value * fs * em_factor(l.unit),
                        })
                        .map_or(0.0, |v| v as f32)
                },
                |p| (span - dim) * p / 100.0,
            ),
    }
}

/// Longhand mono-axe `background-position-x`/`-y`
/// (css-backgrounds-4 §3.5, R-261) → décalage (px) de la
/// tuile sur cet axe : `center`, `bord` (seul ou avec offset
/// — `left 10px`, `right 10px`), `<len>`/`<%>` nu mesuré
/// depuis le bord de début. `x-start`/`y-start` = bord de
/// début, `x-end`/`y-end` = bord de fin (approximation
/// LTR/TB — le repère d'écriture n'est pas porté ici).
/// `None` = absent ou hors grammaire → l'appelant garde
/// l'axe du shorthand combiné.
#[allow(clippy::cast_possible_truncation)]
fn bg_axis_position(raw: Option<&str>, span: f32, dim: f32, fs: f64) -> Option<f32> {
    let toks: Vec<&str> = raw?.split_whitespace().collect();
    // `<len>`/`<%>` → px : le `%` mesure (zone − image) comme
    // `bg_position` ; les longueurs font-relatives sur `fs`.
    let len_px = |t: &str| -> Option<f32> {
        if let Some(p) = t.strip_suffix('%') {
            return p.parse::<f32>().ok().map(|n| (span - dim) * n / 100.0);
        }
        parse_length(t)
            .map(|l| match l.unit.px_factor() {
                Some(f) => l.value * f,
                None => l.value * fs * em_factor(l.unit),
            })
            .map(|v| v as f32)
    };
    match toks.as_slice() {
        [a] => match *a {
            "left" | "top" | "x-start" | "y-start" => Some(0.0),
            "center" => Some((span - dim) / 2.0),
            "right" | "bottom" | "x-end" | "y-end" => Some(span - dim),
            t => len_px(t),
        },
        // Forme `bord offset` : l'offset se mesure depuis le
        // bord nommé (`right 10px` = bord droit − 10 px).
        [a, b] => {
            let off = len_px(b)?;
            match *a {
                "left" | "top" | "x-start" | "y-start" => Some(off),
                "right" | "bottom" | "x-end" | "y-end" => Some(span - dim - off),
                "center" => Some((span - dim) / 2.0),
                t => len_px(t),
            }
        }
        _ => None,
    }
}

/// Plafond de la tuile `background-size` mise à l'échelle
/// (R-95) — 16 Mpx ≈ le budget `image_max_px` des décodages ;
/// au-delà, la tuile naturelle est conservée (dégradation
/// tolérante, jamais d'allocation hostile).
const MAX_TILE_PX: u64 = 16_777_216;

/// `background-size` → dimensions (px) d'une tuile dans la
/// zone `area` (R-95) : `auto`/absent = **taille
/// intrinsèque** (spec — la tuile était historiquement
/// naturelle) ; `cover`/`contain` (ratio préservé),
/// `<len>`/`<%>` (une valeur = largeur, l'autre garde le
/// ratio). Même grammaire que [`mask_size`], seul le défaut
/// `auto` diffère (étirement pour les masks — déviation
/// documentée R-52). Longueurs font-relatives résolues sur
/// `fs` comme `background-position`.
#[allow(clippy::cast_possible_truncation)]
// le `as f32` des longueurs suit le même motif que
// `mask_size`/`bg_position` — valeurs CSS ordinaires.
fn bg_size(raw: Option<&str>, area: (f32, f32), img: (f32, f32), fs: f64) -> (f32, f32) {
    let toks: Vec<&str> = raw.unwrap_or("").split_whitespace().collect();
    let ratio = if img.1 > 0.0 { img.0 / img.1 } else { 0.0 };
    let len = |t: &str, span: f32| -> Option<f32> {
        t.strip_suffix('%')
            .and_then(|p| p.parse::<f32>().ok())
            .map(|p| span * p / 100.0)
            .or_else(|| {
                parse_length(t).map(|l| {
                    (match l.unit.px_factor() {
                        Some(f) => l.value * f,
                        None => l.value * fs * em_factor(l.unit),
                    }) as f32
                })
            })
    };
    match toks.as_slice() {
        [] | ["auto"] | ["auto", "auto"] => img,
        ["cover" | "contain"] => {
            if img.0 <= 0.0 || img.1 <= 0.0 {
                return img;
            }
            let s = if toks[0] == "cover" {
                (area.0 / img.0).max(area.1 / img.1)
            } else {
                (area.0 / img.0).min(area.1 / img.1)
            };
            (img.0 * s, img.1 * s)
        }
        [a] => len(a, area.0).map_or(img, |w| (w, if ratio > 0.0 { w / ratio } else { img.1 })),
        [a, b, ..] => {
            let w = if *a == "auto" {
                len(b, area.1).map_or(img.0, |h| h * ratio)
            } else {
                len(a, area.0).unwrap_or(img.0)
            };
            let h = if *b == "auto" {
                len(a, area.0).map_or(img.1, |w| if ratio > 0.0 { w / ratio } else { img.1 })
            } else {
                len(b, area.1).unwrap_or(img.1)
            };
            (w, h)
        }
    }
}

/// Tuile `background-size` mise à l'échelle en nearest-
/// neighbor (R-95) — le tuilage échantillonne ensuite la
/// copie réduite à l'unité près, comme une image native de
/// cette taille. `w`/`h` ≥ 1 garantis par l'appelant.
#[allow(clippy::cast_possible_truncation)]
// w×h ≤ MAX_TILE_PX (16 Mpx) vérifié avant l'appel — chaque
// dimension tient largement dans u32.
fn scale_tile(img: &crate::image::DecodedImage, w: u32, h: u32) -> crate::image::DecodedImage {
    let (iw, ih) = (img.width as usize, img.height as usize);
    let (w, h) = (w as usize, h as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for dy in 0..h {
        let sy = dy * ih / h;
        for dx in 0..w {
            let sx = dx * iw / w;
            let i = (sy * iw + sx) * 4;
            rgba.extend_from_slice(&img.rgba[i..i + 4]);
        }
    }
    crate::image::DecodedImage {
        width: w as u32,
        height: h as u32,
        rgba,
    }
}

/// `mask-size` → dimensions (px) d'une tuile du mask dans la
/// zone `area` (border box). Couvre le motif réel :
/// `cover`/`contain` (ratio préservé), `<len>`/`<%>` (une
/// valeur = largeur, l'autre garde le ratio), `auto` ou
/// absent = **étirement sur la zone** — déviation assumée à
/// la spec (qui veut la taille intrinsèque) : le motif
/// « icône colorée » dimensionne toujours sa boîte à
/// l'icône, et l'étirement était le comportement R-52 dont
/// les pixels sont la référence. Longueurs font-relatives
/// résolues sur `fs` comme `background-position`.
#[allow(clippy::cast_possible_truncation)]
fn mask_size(
    st: &explorer_css::ComputedStyle,
    area: (f32, f32),
    img: (f32, f32),
    fs: f64,
) -> (f32, f32) {
    let raw = st.get("mask-size").or_else(|| st.get("-webkit-mask-size"));
    let toks: Vec<&str> = raw.unwrap_or("").split_whitespace().collect();
    // `img.1 > 0` garanti par l'appelant (bitmap décodée).
    let ratio = img.0 / img.1;
    let len = |t: &str, span: f32| -> Option<f32> {
        t.strip_suffix('%')
            .and_then(|p| p.parse::<f32>().ok())
            .map(|p| span * p / 100.0)
            .or_else(|| {
                parse_length(t).map(|l| {
                    (match l.unit.px_factor() {
                        Some(f) => l.value * f,
                        None => l.value * fs * em_factor(l.unit),
                    }) as f32
                })
            })
    };
    match toks.as_slice() {
        // `auto`/absent : étirement (déviation — voir doc).
        [] | ["auto"] | ["auto", "auto"] => area,
        ["cover" | "contain"] => {
            if img.0 <= 0.0 || img.1 <= 0.0 {
                return area;
            }
            let s = if toks[0] == "cover" {
                (area.0 / img.0).max(area.1 / img.1)
            } else {
                (area.0 / img.0).min(area.1 / img.1)
            };
            (img.0 * s, img.1 * s)
        }
        [a] => len(a, area.0).map_or(area, |w| (w, if ratio > 0.0 { w / ratio } else { w })),
        [a, b, ..] => {
            let w = if *a == "auto" {
                len(b, area.1).map_or(img.0, |h| h * ratio)
            } else {
                len(a, area.0).unwrap_or(img.0)
            };
            let h = if *b == "auto" {
                len(a, area.0).map_or(img.1, |w| if ratio > 0.0 { w / ratio } else { w })
            } else {
                len(b, area.1).unwrap_or(img.1)
            };
            (w, h)
        }
    }
}

/// `mask-image` résolu pour `bx` → `(bitmap, tuile, repeat)`
/// en coords document. Lookup R-52 : `NodeId` d'abord, repli
/// par texte `url(...)` (`masks_url`) pour les boîtes pseudo
/// sans `NodeId` et les masks fetchés par l'appelant. `rect` =
/// border box **décalée** (`mask-clip` initial) ; `dst` porte
/// `mask-size`/`mask-position` résolus. Partagé par
/// `MaskedFill` (feuille « icône ») et `PushMask` (groupe
/// R-252) — un seul site de résolution.
/// Aire rectangulaire d'une boîte de référence mask
/// (`border-box`/`padding-box`/`content-box`) — R-252
/// `mask-origin`/`mask-clip`. `fill-box`/`stroke-box`/
/// `view-box` (référence SVG) et `no-clip`/`margin-box`
/// approchés : `fill|stroke|view|no-clip` → border box,
/// `margin-box` → border box élargie des marges.
fn mask_area(bx: &LayoutBox, kind: &str) -> Rect {
    let st = &bx.style;
    match kind {
        "padding-box" => {
            let l = border_side_w(st, "left");
            let t = border_side_w(st, "top");
            let r = border_side_w(st, "right");
            let b = border_side_w(st, "bottom");
            Rect {
                x: bx.rect.x + l,
                y: bx.rect.y + t,
                width: (bx.rect.width - l - r).max(0.0),
                height: (bx.rect.height - t - b).max(0.0),
            }
        }
        "content-box" => content_rect(bx),
        "margin-box" => {
            let ml = prop_px(st, "margin-left", 0.0);
            let mt = prop_px(st, "margin-top", 0.0);
            let mr = prop_px(st, "margin-right", 0.0);
            let mb = prop_px(st, "margin-bottom", 0.0);
            Rect {
                x: bx.rect.x - ml,
                y: bx.rect.y - mt,
                width: bx.rect.width + ml + mr,
                height: bx.rect.height + mt + mb,
            }
        }
        _ => bx.rect,
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn resolve_mask(bx: &LayoutBox, rect: &Rect, images: &crate::PaintImages) -> Vec<MaskLayer> {
    // Couches `mask-image` : texte → bitmap ; la couche 0 garde
    // le lookup `NodeId` historique, les suivantes passent par
    // `masks_url` (multi-couches R-252, union `add`).
    let urls: Vec<String> = bx
        .style
        .get("mask-image")
        .or_else(|| bx.style.get("-webkit-mask-image"))
        .map(|v| top_commas(v).filter_map(crate::dataurl::url_arg).collect())
        .unwrap_or_default();
    let (ox, oy) = (rect.x - bx.rect.x, rect.y - bx.rect.y);
    let shift = |mut a: Rect| {
        a.x += ox;
        a.y += oy;
        a
    };
    let clip_kind = bx
        .style
        .get("mask-clip")
        .or_else(|| bx.style.get("-webkit-mask-clip"))
        .map_or("border-box", str::trim);
    let origin_kind = bx
        .style
        .get("mask-origin")
        .or_else(|| bx.style.get("-webkit-mask-origin"))
        .map_or("border-box", str::trim);
    let origin = shift(mask_area(bx, origin_kind));
    let clip = shift(mask_area(bx, clip_kind));
    let area = (origin.width, origin.height);
    let fs = font_size_of(&bx.style);
    let mut out = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        let mask = if i == 0 {
            bx.node
                .and_then(|n| images.masks.get(&n))
                .or_else(|| images.masks_url.get(url))
        } else {
            images.masks_url.get(url)
        };
        let Some(mask) = mask else { continue };
        // Longhands à couches : i-ème valeur (ou dernière —
        // répétition spec) ; la couche 0 reprend le helper
        // historique pour les valeurs simples.
        let size_raw = if i == 0 {
            None
        } else {
            mask_layer_prop(&bx.style, "mask-size", i)
        };
        let (mw, mh) = if let Some(r) = size_raw {
            mask_size_raw(r, area, (mask.width as f32, mask.height as f32), fs)
        } else {
            mask_size(&bx.style, area, (mask.width as f32, mask.height as f32), fs)
        };
        let pos_raw = mask_layer_prop(&bx.style, "mask-position", i);
        let (px, py) = bg_position(pos_raw, area, (mw, mh), fs);
        out.push(MaskLayer {
            mask: mask.clone(),
            dst: Rect {
                x: origin.x + px,
                y: origin.y + py,
                width: mw,
                height: mh,
            },
            clip,
            repeat: mask_layer_prop(&bx.style, "mask-repeat", i)
                .map_or_else(|| mask_repeat(&bx.style), mask_repeat_raw),
            luminance: mask_layer_prop(&bx.style, "mask-mode", i).is_some_and(|v| v == "luminance"),
            composite: mask_layer_prop(&bx.style, "mask-composite", i).map_or(0, |v| match v {
                "intersect" => 1,
                "subtract" => 2,
                "exclude" => 3,
                _ => 0,
            }),
        });
    }
    out
}

/// `mask-repeat` d'une couche → `(x, y)` — même table que
/// [`mask_repeat`] sur un fragment déjà isolé.
fn mask_repeat_raw(raw: &str) -> (u8, u8) {
    let toks: Vec<&str> = raw.split_whitespace().collect();
    // Modes css-masking-1 §7.2 : 0 repeat, 1 no-repeat,
    // 2 space, 3 round.
    let mode = |t: &str| match t {
        "no-repeat" => 1u8,
        "space" => 2,
        "round" => 3,
        _ => 0,
    };
    match toks.as_slice() {
        [] => (0, 0),
        ["repeat-x"] => (0, 1),
        ["repeat-y"] => (1, 0),
        [a] => (mode(a), mode(a)),
        [a, b, ..] => (mode(a), mode(b)),
    }
}

/// `mask-size` d'une couche → dimensions de la tuile — même
/// grammaire que [`mask_size`] sur un fragment déjà isolé.
#[allow(clippy::cast_possible_truncation)]
fn mask_size_raw(raw: &str, area: (f32, f32), img: (f32, f32), fs: f64) -> (f32, f32) {
    // Réutilise `mask_size` via un style temporaire impossible
    // — version fragment : mêmes branches sur tokens simples.
    let toks: Vec<&str> = raw.split_whitespace().collect();
    let ratio = if img.1 > 0.0 { img.0 / img.1 } else { 1.0 };
    let len = |t: &str, span: f32| -> Option<f32> {
        t.strip_suffix('%')
            .and_then(|p| p.parse::<f32>().ok())
            .map(|p| span * p / 100.0)
            .or_else(|| {
                parse_length(t).map(|l| {
                    (match l.unit.px_factor() {
                        Some(f) => l.value * f,
                        None => l.value * fs * em_factor(l.unit),
                    }) as f32
                })
            })
    };
    match toks.as_slice() {
        [] | ["auto"] | ["auto", "auto"] => area,
        ["cover" | "contain"] => {
            if img.0 <= 0.0 || img.1 <= 0.0 {
                return area;
            }
            let s = if toks[0] == "cover" {
                (area.0 / img.0).max(area.1 / img.1)
            } else {
                (area.0 / img.0).min(area.1 / img.1)
            };
            (img.0 * s, img.1 * s)
        }
        [a] => len(a, area.0).map_or(area, |w| (w, if ratio > 0.0 { w / ratio } else { w })),
        [a, b, ..] => {
            let w = if *a == "auto" {
                len(b, area.1).map_or(img.0, |h| h * ratio)
            } else {
                len(a, area.0).unwrap_or(img.0)
            };
            let h = if *b == "auto" {
                len(a, area.0).map_or(img.1, |w| if ratio > 0.0 { w / ratio } else { w })
            } else {
                len(b, area.1).unwrap_or(img.1)
            };
            (w, h)
        }
    }
}

/// i-ème valeur d'une longhand `mask-*` à couches — liste
/// virgule top-level ; hors-portée → `None` (le site retombe
/// sur la dernière — répétition spec — via `or_else` amont).
fn mask_layer_prop<'a>(
    st: &'a explorer_css::ComputedStyle,
    name: &str,
    i: usize,
) -> Option<&'a str> {
    let raw = st
        .get(name)
        .or_else(|| st.get(&format!("-webkit-{name}")))?;
    let parts: Vec<&str> = top_commas(raw).collect();
    parts
        .get(i)
        .or_else(|| parts.last())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
}

/// Virgules top-level (parenthèses ignorées).
fn top_commas(raw: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0u32;
    let mut start = 0usize;
    let mut out = Vec::new();
    for (i, ch) in raw.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&raw[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&raw[start..]);
    out.into_iter()
}

/// Une tuile de mask (R-252 multi-couches) : bitmap,
/// `dst`/`repeat` résolus, `luminance`.
#[derive(Debug, Clone)]
pub struct MaskTile {
    /// Bitmap — seul l'alpha (ou la luma) est lu.
    pub mask: std::sync::Arc<crate::image::DecodedImage>,
    /// Zone d'une tuile du mask (coords document).
    pub dst: Rect,
    /// `mask-repeat` `(x, y)`.
    pub repeat: (u8, u8),
    /// `mask-mode:luminance` de la couche.
    pub luminance: bool,
    /// `mask-composite` de la couche : 0 `add` (initial —
    /// union max), 1 `intersect` (min), 2 `subtract`
    /// (`d×(1−cov)`), 3 `exclude` (xor).
    pub composite: u8,
}

/// Résolution d'une couche `mask-*` (R-252) : bitmap dont
/// l'alpha (ou la luminance) masque, tuile `dst`
/// (origin+size+position) et aire de peinture `clip` —
/// coordonnées document.
struct MaskLayer {
    /// Bitmap — seul l'alpha est lu (luminance : luma×alpha).
    mask: std::sync::Arc<crate::image::DecodedImage>,
    /// Tuile résolue (`mask-origin`+`mask-size`+`mask-position`).
    dst: Rect,
    /// Aire de peinture (`mask-clip`) — hors d'elle, alpha 0.
    clip: Rect,
    /// `mask-repeat` `(x, y)` — mode u8 (0/1/2/3).
    repeat: (u8, u8),
    /// `mask-mode:luminance` résolu.
    luminance: bool,
    /// `mask-composite` de la couche (0 add…3 exclude).
    composite: u8,
}

/// `mask-repeat` → `(x, y)` : `repeat` par défaut (tuilage),
/// `space`/`round` approchés en `repeat` (nearest — l'écart
/// de rendu est sub-pixel sur les motifs icon, où la tuile
/// couvre la zone). Deux valeurs = x puis y.
/// `border-image-slice` : insets `(t, r, b, l)` en pixels
/// source — `%` rapportés à la dimension de l'image
/// (t/b → hauteur, r/l → largeur), nombre nu = px.
/// `fill` est lu à l'émission (pas un inset).
fn border_img_slice(st: &explorer_css::ComputedStyle) -> [f32; 4] {
    // `border-image-slice` : insets `(t, r, b, l)` —
    // nombre nu = pixels source, `%` = pourcentage de la
    // dimension image (t/b → hauteur, r/l → largeur).
    // Convention interne : `≥0` = px, `<0` = -%×100
    // (le raster décode contre iw/ih). `fill` lu ailleurs.
    let v = st.get("border-image-slice").unwrap_or_default();
    let mut out = [0.0f32; 4];
    let vals: Vec<&str> = v.split_whitespace().filter(|p| *p != "fill").collect();
    for (i, o) in out.iter_mut().enumerate() {
        let t = vals
            .get(i)
            .or_else(|| vals.first())
            .copied()
            .unwrap_or("100%");
        *o = if let Some(n) = t.strip_suffix('%') {
            -n.parse::<f32>().unwrap_or(0.0)
        } else {
            t.parse::<f32>().unwrap_or(0.0)
        };
    }
    out
}

/// `border-image-width` : épaisseurs `(t, r, b, l)` px —
/// `auto`/absent → les `border-*-width` calculés.
fn border_img_widths(st: &explorer_css::ComputedStyle, fallback: [f32; 4]) -> [f32; 4] {
    let v = st.get("border-image-width");
    let mut out = fallback;
    if let Some(v) = v {
        let parts: Vec<&str> = v.split_whitespace().collect();
        for (i, o) in out.iter_mut().enumerate() {
            if let Some(t) = parts.get(i).or_else(|| parts.first()) {
                if let Some(px) = t.strip_suffix("px").and_then(|n| n.parse::<f32>().ok()) {
                    *o = px;
                } else if let Ok(n) = t.parse::<f32>() {
                    *o = n; // multiplicateur de border-width — simplification.
                }
            }
        }
    }
    out
}

/// `border-image-repeat` — 0 stretch, 1 repeat, 2 round, 3 space.
fn border_img_repeat(st: &explorer_css::ComputedStyle) -> (u8, u8) {
    let v = st.get("border-image-repeat").unwrap_or("stretch");
    let mut it = v.split_whitespace();
    let m = |t: &str| match t {
        "repeat" => 1u8,
        "round" => 2u8,
        "space" => 3u8,
        _ => 0u8,
    };
    (
        m(it.next().unwrap_or("stretch")),
        m(it.next().unwrap_or("stretch")),
    )
}

fn mask_repeat(st: &explorer_css::ComputedStyle) -> (u8, u8) {
    let raw = st
        .get("mask-repeat")
        .or_else(|| st.get("-webkit-mask-repeat"));
    let toks: Vec<&str> = raw.unwrap_or("").split_whitespace().collect();
    // Modes css-masking-1 §7.2 : 0 repeat, 1 no-repeat,
    // 2 space, 3 round.
    let mode = |t: &str| match t {
        "no-repeat" => 1u8,
        "space" => 2,
        "round" => 3,
        _ => 0,
    };
    match toks.as_slice() {
        [] => (0, 0),
        ["repeat-x"] => (0, 1),
        ["repeat-y"] => (1, 0),
        [a] => (mode(a), mode(a)),
        [a, b, ..] => (mode(a), mode(b)),
    }
}

/// `true` si le fragment ne rend aucun glyphe : texte réduit
/// à des blancs CSS collapsibles (un atome `inline-block`/
/// remplacé EST du contenu — `\u{A0}` aussi, cf. `trim_css_ws`).
fn frag_is_ws_only(f: &explorer_layout::inline::Fragment) -> bool {
    !f.atomic && trim_css_ws(&f.text).is_empty()
}

/// `true` si la boîte est du blanc collapsible : texte tout
/// blanc, ou bloc anonyme dont toute la descendance est
/// blanche — sert au critère « cellule vide » de
/// `empty-cells:hide` (§17.6.1.1, R-263).
fn box_is_ws_only(b: &LayoutBox) -> bool {
    match b.kind {
        BoxKind::Text => b.text.as_deref().is_none_or(|t| trim_css_ws(t).is_empty()),
        BoxKind::AnonymousBlock => {
            b.children.iter().all(box_is_ws_only)
                && b.lines
                    .iter()
                    .all(|l| l.fragments.iter().all(frag_is_ws_only))
        }
        _ => false,
    }
}

/// Fond + bordures + texte propre d'une boîte (avant les
/// enfants — le fond passe TOUJOURS sous son contenu).
/// `off` = décalage vertical de scroll (`0` dans les
/// sous-arbres `position: fixed`).
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
fn paint_self(
    bx: &LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    vp: (f32, f32),
    images: &crate::PaintImages,
    canvas_bg_src: Option<explorer_dom::NodeId>,
) {
    let rect = shifted(bx.rect, off);
    // Blocs anonymes et boîtes texte clonent le style du
    // parent, mais `background`/`border` ne sont PAS hérités
    // en CSS : ils ne peignent ni fond ni bordure propre
    // (sinon le clone masquerait les flottants posés avant,
    // ou la bordure du parent serait redessinée sur chaque
    // fragment de texte — §9.2.1.1).
    let anonymous = matches!(bx.kind, BoxKind::AnonymousBlock | BoxKind::Text);
    // Boîte `display:inline` pure (ni atome ni remplacé) :
    // `bx.rect` n'est que l'UNION de ses fragments — un fond
    // ou une bordure peint dessus produirait un blob couvrant
    // le texte voisin (R-22). Sa décoration est émise par
    // fragment dans la boucle des lignes (voir plus bas).
    // `display()` retombe sur "block" pour un élément sans
    // déclaration — le discriminant fiable est `kind` : une
    // boîte `Inline` non atomique (pas `inline-block`, pas
    // remplacée) est un pur conteneur de flux en-ligne.
    let inline_flow = is_inline_flow(bx);
    // Fond : `background-color` non transparente, puis
    // `background-image` décodée (au-dessus de la couleur,
    // sous la bordure — App. E ; origine padding box ≈ bord
    // interne de la bordure gauche/haute). Sautée quand le
    // fond de cette boîte a été propagé au canvas (§14.2) —
    // `canvas_bg_src` l'a déjà peint sur tout le viewport.
    // La comparaison porte sur le nœud : un `None` (boîte
    // pseudo `::before`/`::after`, anonyme) n'est JAMAIS la
    // source propagée — `None != None` sautait le fond de
    // tout pseudo (`:after{…gradient}` text.npr.org, R-74).
    let bg_propagated = bx.node.is_some_and(|n| canvas_bg_src == Some(n));
    // R-164 — `empty-cells:hide` (§17.6.1.1) : sous le
    // modèle `separate`, une cellule sans contenu (ni
    // enfants ni lignes — `&nbsp;` compte comme contenu)
    // ne peint ni fond ni bordure.
    // R-263 : une cellule réduite à des blancs collapsibles
    // (`<td> </td>` — espace, tab, saut) est « vide » au
    // sens de la spec : les boîtes texte/blocs anonymes qui
    // ne portent que du blanc CSS ne comptent pas comme
    // contenu (`\u{A0}` si — il rend de l'encre).
    let empty_hidden = bx
        .style
        .get("display")
        .is_some_and(|v| v.trim() == "table-cell")
        && bx
            .style
            .get("empty-cells")
            .is_some_and(|v| v.trim() == "hide")
        && bx
            .style
            .get("border-collapse")
            .is_none_or(|v| v.trim() != "collapse")
        && bx.children.iter().all(box_is_ws_only)
        && bx
            .lines
            .iter()
            .all(|l| l.fragments.iter().all(frag_is_ws_only));
    // R-M9b — `backdrop-filter` : la zone déjà peinte
    // sous la boîte reçoit les ops AVANT que fond/contenu
    // ne s'affichent par-dessus (raster : extract →
    // ops → recompose la région).
    if !anonymous && !inline_flow {
        let bops = bx.style.get("backdrop-filter");
        if let Some(v) = bops {
            if !v.trim().eq_ignore_ascii_case("none") {
                let mut st = bx.style.clone();
                // `parse_filter` lit `filter` — même
                // grammaire `<filter-function-list>` :
                // injecter la valeur `backdrop-filter`.
                st.set("filter", v);
                let ops = parse_filter(&st);
                if !ops.is_empty() {
                    items.push(DisplayItem::BackdropFilter { rect, ops });
                }
            }
        }
    }
    // R-M2 — `box-shadow` : une ombre par couche, émise
    // AVANT le fond de la boîte (sous lui dans l'ordre de
    // peinture). `inset` non porté (bordure interne
    // d'ombre = approximation documentée, sautée).
    if !anonymous && !inline_flow && !empty_hidden {
        emit_box_shadows(&bx.style, &rect, items);
    }
    if !anonymous && !inline_flow && !bg_propagated && !empty_hidden {
        if let Some(c) = bx.style.color("background-color") {
            if c.a > 0 {
                // R-52 — `mask-image` présent : la couleur ne
                // passe que par l'alpha du mask (icônes des
                // design systems). Mask absent des tables
                // (non décodé/réseau) → repli fond plein,
                // comportement antérieur. Repli **par url**
                // (`masks_url`) : les boîtes pseudo n'ont pas
                // de `NodeId` — leur mask se retrouve par le
                // texte `url(...)` du style (motif `.x:before`
                // des icônes MDN), et les masks fetchés sont
                // enregistrés par l'appelant sur la même clé.
                // R-252 — boîte à contenu (enfants/lignes) :
                // le `PushMask` ancêtre masque déjà le groupe
                // — fond plein ici, un `MaskedFill`
                // re-masquerait (alpha² sur les bords doux).
                let group_masked = !bx.children.is_empty() || !bx.lines.is_empty();
                let mut mask_layers = if group_masked {
                    Vec::new()
                } else {
                    resolve_mask(bx, &rect, images)
                };
                // Feuille **multi-couches** : `MaskedFill` ne
                // porte qu'une couche — bracket groupe autour
                // du remplissage, la couverture du groupe fait
                // l'union des couches (R-252 suite).
                let pushed_group = mask_layers.len() > 1;
                if pushed_group {
                    items.push(DisplayItem::PushMask {
                        rect,
                        layers: mask_layers
                            .iter()
                            .map(|l| MaskTile {
                                mask: l.mask.clone(),
                                dst: l.dst,
                                repeat: l.repeat,
                                luminance: l.luminance,
                                composite: l.composite,
                            })
                            .collect(),
                    });
                }
                if let Some(l) = if mask_layers.is_empty() {
                    None
                } else {
                    Some(mask_layers.remove(0))
                } {
                    items.push(DisplayItem::MaskedFill {
                        rect,
                        color: c,
                        mask: l.mask,
                        dst: l.dst,
                        repeat: l.repeat,
                        luminance: l.luminance,
                    });
                    if pushed_group {
                        items.push(DisplayItem::PopMask);
                    }
                } else if bx
                    .style
                    .get("background-clip")
                    .is_some_and(|v| v.split(',').any(|p| p.trim() == "text"))
                {
                    // R-184 — `background-clip:text` : fond
                    // solide peint une fois par fragment de
                    // ligne (même approximation que les
                    // couches image — `PushClip` par run).
                    for fr in text_clip_rects(bx) {
                        {
                            let f = shifted(fr, off);
                            items.push(DisplayItem::PushClip(f));
                            items.push(DisplayItem::FillRect {
                                rect,
                                color: c,
                                radii: corner_radii(&bx.style, &rect),
                                corners: corner_ks(&bx.style),
                            });
                            items.push(DisplayItem::PopClip);
                        }
                    }
                } else {
                    items.push(DisplayItem::FillRect {
                        rect,
                        color: c,
                        radii: corner_radii(&bx.style, &rect),
                        corners: corner_ks(&bx.style),
                    });
                }
            }
        }
        // Zone de positionnement : **padding box** (bord interne
        // des bordures — `background-position` s'y réfère,
        // §14.2.1) pour `scroll`, viewport pour `fixed` (le
        // fond reste calé à l'écran). L'origine est en coords
        // FRAME : le viewport y démarre en (0,0) — ancrer au
        // scroll déphaserait la tuile de `scroll mod taille`.
        // La zone PEINTE reste la border box (clip par défaut)
        // — seule l'ancre de la tuile bouge.
        let fixed = bx.style.get("background-attachment") == Some("fixed");
        let (area, base) = if fixed {
            ((vp.0, vp.1), (0.0, 0.0))
        } else {
            let bl = border_side_w(&bx.style, "left");
            let bt = border_side_w(&bx.style, "top");
            let br = border_side_w(&bx.style, "right");
            let bb = border_side_w(&bx.style, "bottom");
            (
                (
                    (rect.width - bl - br).max(0.0),
                    (rect.height - bt - bb).max(0.0),
                ),
                (rect.x + bl, rect.y + bt),
            )
        };
        emit_bg_layers(items, bx, images, rect, area, base, off);
    }
    // Bordures : 4 côtés, épaisseur > 0 et style non `none`.
    // Sémantique de `border_side_w` : la largeur ne compte que
    // si le style du côté est déclaré non-`none` — l'initiale
    // `medium` vaut alors 3 px (`border: solid` sans épaisseur
    // peint 3 px).
    let widths = if anonymous || inline_flow || empty_hidden {
        [0.0; 4]
    } else {
        [
            border_side_w(&bx.style, "top"),
            border_side_w(&bx.style, "right"),
            border_side_w(&bx.style, "bottom"),
            border_side_w(&bx.style, "left"),
        ]
    };
    if widths.iter().any(|v| *v > 0.0) {
        let color_of = |side: &str| border_color_of(&bx.style, side);
        items.push(DisplayItem::Border {
            rect,
            widths,
            colors: [
                color_of("border-top-color"),
                color_of("border-right-color"),
                color_of("border-bottom-color"),
                color_of("border-left-color"),
            ],
            styles: [
                border_style(bx.style.get("border-top-style")),
                border_style(bx.style.get("border-right-style")),
                border_style(bx.style.get("border-bottom-style")),
                border_style(bx.style.get("border-left-style")),
            ],
            radii: corner_radii(&bx.style, &rect),
            corners: corner_ks(&bx.style),
        });
    }
    // R-248 — `border-image` : source url résolue par
    // `borders_url` (data-url collectés, fetchés par
    // `register_border_url`) → découpe 9 régions émise après
    // la bordure classique (le style `border-image-*`
    // **remplace** le visuel des côtés — l'item `Border` ci-
    // dessus reste pour les coins/rayons et les bords non
    // couverts par l'image).
    if !anonymous && !inline_flow && !empty_hidden {
        if let Some(img) =
            crate::dataurl::border_img_url(&bx.style).and_then(|u| images.borders_url.get(&u))
        {
            items.push(DisplayItem::BorderImage {
                rect,
                img: img.clone(),
                src_edges: border_img_slice(&bx.style),
                dst_edges: border_img_widths(&bx.style, widths),
                fill_center: bx
                    .style
                    .get("border-image-slice")
                    .is_some_and(|v| v.split_whitespace().any(|p| p == "fill")),
                repeat: border_img_repeat(&bx.style),
            });
        }
    }
    // `outline` (R-M13) : anneau autour de la bordure, sans
    // emprise au layout — peint hors border box à
    // `outline-offset` + `outline-width` de l'edge externe.
    // Style `none` → rien ; épaisseur initiale `medium` = 3 px
    // (même convention que border). Rayons suivis via
    // `corner_radii` sur le rect étendu.
    if !anonymous && !inline_flow {
        let ow_style = border_style(bx.style.get("outline-style"));
        if !matches!(ow_style, BorderStyle::None) {
            let ow = prop_px(&bx.style, "outline-width", 3.0).max(0.0);
            let off = prop_px(&bx.style, "outline-offset", 0.0);
            if ow > 0.0 {
                let grow = ow + off;
                let orect = Rect {
                    x: rect.x - grow,
                    y: rect.y - grow,
                    width: rect.width + 2.0 * grow,
                    height: rect.height + 2.0 * grow,
                };
                let oc = border_color_of(&bx.style, "outline-color");
                items.push(DisplayItem::Border {
                    rect: orect,
                    widths: [ow; 4],
                    colors: [oc; 4],
                    styles: [ow_style; 4],
                    radii: corner_radii(&bx.style, &orect),
                    corners: corner_ks(&bx.style),
                });
            }
        }
    }
    // (Les fragments `bx.lines` sont émis par `paint_lines`,
    // appelée dans la zone clippée — voir `paint_box` / R-55.)
    // Contrôle coché (`checkbox`/`radio`, étape 38) : marque
    // pleine centrée — carré à ~55 % du plus petit côté,
    // couleur = encre de la boîte (approx MVP documentée).
    if bx.control_checked {
        let side = (rect.width.min(rect.height) * 0.55).max(0.0);
        if side > 0.0 {
            // `accent-color` prime sur `color` pour la marque
            // de contrôle (css-ui-4 §accent-color).
            let ink = bx
                .style
                .color("accent-color")
                .or_else(|| bx.style.color("color"))
                .unwrap_or(Color::BLACK);
            items.push(DisplayItem::FillRect {
                rect: Rect {
                    x: rect.x + (rect.width - side) / 2.0,
                    y: rect.y + (rect.height - side) / 2.0,
                    width: side,
                    height: side,
                },
                color: ink,
                radii: [0.0; 4],
                corners: [1.0; 4],
            });
        }
    }
}

/// Fragments de texte des lignes propres de `bx` — le contenu
/// inline de la boîte quand elle est racine d'un contexte
/// inline (élément inline positionné/flottant, bloc anonyme).
/// Séparé de `paint_self` : ces fragments sont du CONTENU —
/// `paint_box` les émet dans la zone du clip `overflow`
/// (R-55 : le `.sr-only` de lobste.rs — `position:absolute`,
/// `overflow:hidden`, 1×1 — fuyait son clip : ses
/// fragments sortaient avant le `PushClip`).
///
/// `frag.style` = style de l'élément inline propriétaire
/// (le plus proche du texte) — `a{color}`/`span{color}`
/// colorent leurs runs, pas seulement la boîte (R-18).
/// `true` = boîte `display:inline` pure (ni atomique ni
/// remplacée) : son `rect` n'est que l'union de fragments —
/// sa décoration s'émet par pièce de ligne, pas sur la boîte.
/// R-282 — `text-decoration-skip-spaces` (css-text-decor-4
/// §5.5) : intervalles locaux au fragment où le trait reste
/// peint, blancs exclus — positions de plume des glyphes
/// espaces du [`ShapedRun`] (ordre logique, avance signée
/// pour RTL). `mask` = `DECO_SKIP_START`/`END`/`ALL` :
/// `all` coupe chaque blanc ; `start`/`end` la série de
/// blancs touchant ce bord du fragment (borne : bord de
/// fragment ≈ bord de l'élément décoré — les bits start/
/// end des fragments intérieurs sont effacés à l'appelant,
/// donc un fragment tout-blanc n'arrive ici en bord
/// qu'avec son bit → vidé).
fn deco_keep_ranges(
    text: &str,
    run: &ShapedRun,
    width: f32,
    mask: u8,
    ink_band: Option<(f32, f32)>,
) -> Vec<(f32, f32)> {
    use explorer_layout::inline::{DECO_SKIP_ALL, DECO_SKIP_END, DECO_SKIP_START};
    if width <= 0.0 {
        return Vec::new();
    }
    if mask == 0 && ink_band.is_none() {
        return vec![(0.0, width)];
    }
    // Fragment tout-blanc : vidé dès qu'un saut est demandé.
    // `all` vide partout ; sous `start`/`end` seuls les
    // fragments de bord d'owner gardent leur bit (les
    // intérieurs ont `sk == 0` — première ligne) → un blanc
    // de tête/queue d'élément est sauté, un blanc intérieur
    // gardé (erratum re-tour : `start` laissait le trait
    // sur le fragment tout-blanc de tête).
    if text.chars().all(is_css_ws) {
        return if mask != 0 {
            Vec::new()
        } else {
            vec![(0.0, width)]
        };
    }
    // Étendue de plume de chaque glyphe, + marque blanc.
    let mut pen = 0.0_f32;
    let mut spans: Vec<(f32, f32, bool)> = Vec::with_capacity(run.glyphs.len());
    for g in &run.glyphs {
        let a0 = pen;
        pen += g.advance;
        let is_ws = text
            .get(g.cluster as usize..)
            .and_then(|s| s.chars().next())
            .is_some_and(is_css_ws);
        spans.push((a0.min(pen), a0.max(pen), is_ws));
    }
    // Plages à couper : `all` = tous les blancs ; sinon les
    // séries contiguës en tête (`start`) / queue (`end`).
    let mut cuts: Vec<(f32, f32)> = Vec::new();
    if mask & DECO_SKIP_ALL != 0 {
        cuts.extend(spans.iter().filter(|s| s.2).map(|s| (s.0, s.1)));
    } else {
        if mask & DECO_SKIP_START != 0 {
            for s in &spans {
                if !s.2 {
                    break;
                }
                cuts.push((s.0, s.1));
            }
        }
        if mask & DECO_SKIP_END != 0 {
            for s in spans.iter().rev() {
                if !s.2 {
                    break;
                }
                cuts.push((s.0, s.1));
            }
        }
    }
    // R-286 — `text-decoration-skip-ink` : coupe le trait
    // là où une encre de glyphe traverse la bande.
    // `ink_band` = (haut, bas) du trait dans le repère des
    // `ink_glyphs` (baseline = 0, y positif vers le bas).
    if let Some((top, bot)) = ink_band {
        for r in &run.ink_glyphs {
            if r.width > 0.0 && r.y < bot && r.y + r.height > top {
                cuts.push((r.x, r.x + r.width));
            }
        }
    }
    if cuts.is_empty() {
        return vec![(0.0, width)];
    }
    cuts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    // Soustraction : intervalles conservés dans [0, width].
    let mut keep = Vec::new();
    let mut cur = 0.0_f32;
    for (a, b) in cuts {
        let a = a.clamp(0.0, width);
        let b = b.clamp(0.0, width);
        if a > cur + 0.01 {
            keep.push((cur, a.min(width)));
        }
        cur = cur.max(b);
    }
    if cur < width - 0.01 {
        keep.push((cur, width));
    }
    // Erratum re-tour NAV (régression mesurée) : avec une
    // fonte dont l'encre couvre toute la bande du trait —
    // Ahem (carreau plein sur tout l'em), fontes plein-em —
    // le skip-ink soustrait `DecoRect` entier : le souligné
    // **disparaît** au lieu de trouer aux jambages (7 sondes
    // `text-decoration*` basculées `ok→miss`). `auto` est un
    // « should » : quand l'encre déclarée annihilerait tout,
    // elle est pathologique → trait plein conservé.
    if keep.is_empty() && ink_band.is_some() {
        return vec![(0.0, width)];
    }
    keep
}

fn is_inline_flow(bx: &LayoutBox) -> bool {
    bx.kind == BoxKind::Inline
        && bx.intrinsic.is_none()
        && bx.display() != "inline-block"
        && bx.display() != "inline-flex"
        && bx.display() != "inline-table"
}

/// `text-combine-upright` (css-writing-modes §4.3 — R-288) :
/// mode lu sur le style du fragment — 0 `none`, 1 `all`,
/// 2..=9 = `digits <n>` (séries de n chiffres max). L'alias
/// `-webkit-text-combine` (`none`/`horizontal`) est lu en
/// repli. La propriété n'est pas héritée par spec.
fn combine_mode(style: &explorer_css::ComputedStyle) -> u8 {
    let v = style
        .get("text-combine-upright")
        .or_else(|| style.get("-webkit-text-combine"))
        .map_or("", |v| v.trim());
    if v == "all" || v == "horizontal" {
        return 1;
    }
    if let Some(rest) = v.strip_prefix("digits") {
        // `digits` nu = 2 par spec.
        let n = rest.trim().parse::<u8>().unwrap_or(2).clamp(2, 9);
        return n;
    }
    0
}

/// Groupe `text-combine-upright` débutant au glyphe `gi`
/// (R-288) : série contiguë de glyphes qualifiants — chiffre
/// ASCII pour `digits <n>` (tronçons de `n` max), non-blanc
/// pour `all`. Retourne la fin exclusive si ≥ 2 glyphes.
fn combine_group(run: &ShapedRun, text: &str, gi: usize, mode: u8) -> Option<usize> {
    let qual = |g: &explorer_layout::font::Glyph| {
        text.get(g.cluster as usize..)
            .and_then(|s| s.chars().next())
            .is_some_and(|ch| {
                if mode == 1 {
                    !is_css_ws(ch)
                } else {
                    ch.is_ascii_digit()
                }
            })
    };
    if !qual(&run.glyphs[gi]) {
        return None;
    }
    let mut end = gi + 1;
    if mode == 1 {
        // `all` = série contiguë entière (compressée à 1em).
        while end < run.glyphs.len() && qual(&run.glyphs[end]) {
            end += 1;
        }
    } else {
        // `digits <n>` : tronçons de n chiffres max — un
        // reste de 1 glyphe n'est jamais combiné seul.
        let max = usize::from(mode);
        while end < run.glyphs.len() && end - gi < max && qual(&run.glyphs[end]) {
            end += 1;
        }
    }
    (end - gi >= 2).then_some(end)
}

#[allow(clippy::too_many_lines)]
fn paint_lines(bx: &LayoutBox, items: &mut Vec<DisplayItem>, off: (f32, f32)) {
    // `text-orientation` (css-writing-modes §5.2) : en flux
    // vertical, `sideways`/`sideways-rl`/`sideways-lr`
    // pivote le run entier de ±90° autour de la baseline
    // (rl → +90° horaire, lr → −90°). `upright`/`mixed` =
    // résidus — avance verticale par glyphe hors MVP.
    // NB : `flow_mode` = flux CONTENANT — la boîte verticale
    // elle-même est horizontale pour son parent ; c'est son
    // `writing-mode` propre qui transposé ses lignes.
    let own_v = explorer_css::value::parse_writing_mode(bx.style.get("writing-mode").unwrap_or(""))
        .is_vertical();
    let sideways_lr = own_v
        && matches!(
            bx.style.get("text-orientation").map_or("", |v| v.trim()),
            "sideways" | "sideways-lr"
        );
    let sideways_rl = own_v
        && matches!(
            bx.style.get("text-orientation").map_or("", |v| v.trim()),
            "sideways" | "sideways-rl"
        );
    // `text-orientation:upright` (css-writing-modes §5.2) :
    // glyphes debout empilés verticalement — avance =
    // `font-size` ; `mixed` = résidu (pas de classification
    // par script au niveau glyphe — `upright` explicite seul
    // est consommé).
    // `upright` : tout debout. `mixed` (défaut css) : upright
    // pour les glyphes dont le caractère source (offset
    // `cluster`) est CJK — Han/Hiragana/Katakana/Hangul/
    // pleine-largeur — le reste reste couché (≈ `sideways`,
    // approximation documentée : sans fonte verticale les
    // accents CJK ne sont pas repositionnés).
    let orientation = match bx
        .style
        .get("text-orientation")
        .map_or("mixed", |v| v.trim())
    {
        _ if !own_v => 0u8,
        "upright" => 1,
        "mixed" => 2,
        _ => 0,
    };
    // Deux passes (R-70) : TOUTES les décorations inline
    // d'abord, TOUS les textes ensuite — un fond émis entre
    // deux textes couvrirait l'encre du premier (le fond du
    // bouton-langue Wikipedia mangeait la queue de
    // « language »).
    // R-282 — `text-decoration-skip-spaces` start/end : les
    // vrais bords de l'élément décoré ≈ premier/dernier
    // fragment de son `owner` (enfant direct de `bx`) dans
    // l'ordre document — l'espace intérieur n'est sauté que
    // sous `all` (borne : bord d'élément imbriqué ≈ bord de
    // l'owner de tête, et ordre visuel pour RTL).
    let mut first_of: std::collections::HashMap<usize, (usize, usize)> =
        std::collections::HashMap::new();
    let mut last_of: std::collections::HashMap<usize, (usize, usize)> =
        std::collections::HashMap::new();
    for (li, line) in bx.lines.iter().enumerate() {
        for (fi, f) in line.fragments.iter().enumerate() {
            first_of.entry(f.owner).or_insert((li, fi));
            last_of.insert(f.owner, (li, fi));
        }
    }
    for (li, line) in bx.lines.iter().enumerate() {
        // Décos — une pièce par élément et par ligne (approx.
        // `box-decoration-break: slice`, R-70) : l'union des
        // fragments de l'élément SUR LA LIGNE, padding aux
        // bords de la pièce seulement. `frag.owner` = indice
        // de l'enfant direct de `bx` (même convention que
        // `assign_child_rects`) → tous les fragments d'un
        // élément, descendants compris.
        for (o, child) in bx.children.iter().enumerate() {
            if !is_inline_flow(child) {
                continue;
            }
            let mut union: Option<Rect> = None;
            for f in &line.fragments {
                if f.owner == o {
                    union = Some(match union {
                        None => f.rect,
                        Some(u) => rect_union(u, f.rect),
                    });
                }
            }
            if let Some(u) = union {
                paint_inline_deco(&child.style, u, items, off);
            }
        }
        // Éléments imbriqués (leur fragment porte le style le
        // plus profond — `frag.style` ≠ style de l'enfant
        // direct `owner` et ≠ `bx.style`) : pièce par run
        // contigu de même style, union y comprise.
        let mut run: Option<(Rect, usize)> = None;
        for (i, f) in line.fragments.iter().enumerate() {
            let nested = !f.atomic
                && f.style != bx.style
                && bx.children.get(f.owner).is_some_and(|c| f.style != c.style);
            if !run.is_some_and(|(_, first)| line.fragments[first].style == f.style) {
                if let Some((u, first)) = run.take() {
                    paint_inline_deco(&line.fragments[first].style, u, items, off);
                }
            }
            if nested {
                match &mut run {
                    Some((u, _)) => *u = rect_union(*u, f.rect),
                    r => *r = Some((f.rect, i)),
                }
            }
        }
        if let Some((u, first)) = run {
            paint_inline_deco(&line.fragments[first].style, u, items, off);
        }
        for (fi, frag) in line.fragments.iter().enumerate() {
            if frag.atomic {
                continue;
            }
            if let Some(run) = &frag.shaped {
                let ink = frag.style.color("color").unwrap_or(Color::BLACK);
                let fs = prop_px(&frag.style, "font-size", 16.0);
                // §16.3.1 / R-153 : `text-decoration` résolue
                // à l'itemisation (`frag.deco`, couleur du
                // déclarant). `under`/`over` sous l'encre,
                // `through` après le Text — elle barre les
                // glyphes. Épaisseur ≈ 1/16 d'em, positions
                // usuelles (approx. métriques fonte absente).
                let th = (fs * 0.0625).max(1.0);
                let base_y = frag.rect.y + frag.baseline + off.1;
                // R-280 — `text-decoration-inset` : rentrée
                // du trait aux bords start/end (`%` de la
                // largeur du fragment — base connue ici).
                let deco_rect = |d: &explorer_layout::inline::TextDeco,
                                 dy: f32,
                                 thick: f32,
                                 ink_ok: bool|
                 -> Vec<Rect> {
                    let w = frag.rect.width;
                    let il = if d.inset_l_pct {
                        d.inset_l * w / 100.0
                    } else {
                        d.inset_l
                    };
                    let ir = if d.inset_r_pct {
                        d.inset_r * w / 100.0
                    } else {
                        d.inset_r
                    };
                    // R-282 — `text-decoration-skip-spaces`
                    // : le trait se découpe en segments
                    // hors blancs (`deco_keep_ranges`),
                    // dans les bornes `il..w-ir`. Les
                    // bords start/end ne sautent qu'aux
                    // bords RÉELS de l'élément décoré.
                    let mut sk = d.skip_sp;
                    if first_of.get(&frag.owner) != Some(&(li, fi)) {
                        sk &= !explorer_layout::inline::DECO_SKIP_START;
                    }
                    if last_of.get(&frag.owner) != Some(&(li, fi)) {
                        sk &= !explorer_layout::inline::DECO_SKIP_END;
                    }
                    // R-286 — `skip-ink` : bande du trait
                    // en repère baseline (les `ink_glyphs`
                    // du run vivent baseline = 0).
                    let band = ink_ok.then_some((dy - base_y, dy - base_y + thick));
                    deco_keep_ranges(&frag.text, run, (w - il - ir).max(0.0), sk, band)
                        .into_iter()
                        .map(|(a, b)| Rect {
                            x: frag.rect.x + off.0 + il + a,
                            y: dy,
                            width: (b - a).max(0.0),
                            height: thick,
                        })
                        .collect()
                };
                for d in &frag.deco {
                    let color = d.color.unwrap_or(ink);
                    // R-178 : épaisseur/offset du déclarant
                    // (`text-decoration-thickness` /
                    // `text-underline-offset`) avant repli.
                    let dth = if d.thick_px > 0.0 { d.thick_px } else { th };
                    if d.lines & explorer_layout::inline::DECO_UNDERLINE != 0 {
                        // R-280 — `text-underline-position:under` :
                        // sous les jambages (|descent| + petit
                        // filet) quand l'offset est auto —
                        // `text-underline-offset` déclaré prime.
                        let dy = base_y
                            + d.offset_px.unwrap_or_else(|| {
                                if d.under_pos == 1 {
                                    -run.metrics.descent + (fs * 0.05).max(1.0)
                                } else {
                                    (fs * 0.1).max(1.0)
                                }
                            });
                        for rect in deco_rect(d, dy, dth, d.skip_ink >= 1) {
                            items.push(DisplayItem::DecoRect {
                                rect,
                                color,
                                style: d.style,
                            });
                        }
                    }
                    if d.lines & explorer_layout::inline::DECO_OVERLINE != 0 {
                        for rect in deco_rect(d, base_y - fs * 0.85, dth, d.skip_ink >= 1) {
                            items.push(DisplayItem::DecoRect {
                                rect,
                                color,
                                style: d.style,
                            });
                        }
                    }
                }
                emit_text_shadows(
                    &frag.style,
                    frag.rect.x + off.0,
                    base_y,
                    run,
                    fs,
                    ink,
                    items,
                );
                if sideways_lr || sideways_rl {
                    // Rotation autour de (x, baseline) : lr
                    // −90° (x→−y), rl +90° (x→+y).
                    let (ox, oy) = (frag.rect.x + off.0, base_y);
                    let t = if sideways_lr {
                        tiny_skia::Transform::from_row(0.0, -1.0, 1.0, 0.0, ox - oy, ox + oy)
                    } else {
                        tiny_skia::Transform::from_row(0.0, 1.0, -1.0, 0.0, ox + oy, oy - ox)
                    };
                    items.push(DisplayItem::PushTransform(t));
                }
                if orientation != 0 {
                    // Un `Text` par glyphe, x centré sur la
                    // cellule verticale, y = avance cumulée
                    // depuis le haut du fragment + ascent.
                    // `mixed` : glyphe CJK debout ; glyphe non
                    // CJK → pivote ±90° autour de sa plume.
                    // R-288 — `text-combine-upright` : une
                    // série combinée (digits N / all) est
                    // peinte DEBOUT — un `Text` horizontal
                    // compressé dans 1em (tate-chu-yoko),
                    // occupant un seul créneau vertical.
                    let combine = combine_mode(&frag.style);
                    let mut y = frag.rect.y + run.metrics.ascent;
                    let mut gi = 0usize;
                    while gi < run.glyphs.len() {
                        if combine != 0 {
                            if let Some(end) = combine_group(run, &frag.text, gi, combine) {
                                let sub = &run.glyphs[gi..end];
                                let gw: f32 = sub.iter().map(|g| g.advance).sum();
                                // Compressé à 1em quand la
                                // série dépasse (spec :
                                // tate-chu-yoko tient dans
                                // le corps).
                                let sc = if gw > fs && gw > 0.0 { fs / gw } else { 1.0 };
                                let cx = frag.rect.x + frag.rect.width / 2.0;
                                if sc < 1.0 {
                                    items.push(DisplayItem::PushTransform(
                                        tiny_skia::Transform::from_row(
                                            sc,
                                            0.0,
                                            0.0,
                                            1.0,
                                            cx * (1.0 - sc),
                                            0.0,
                                        ),
                                    ));
                                }
                                let solo = std::sync::Arc::new(ShapedRun {
                                    glyphs: sub.to_vec(),
                                    advance: gw,
                                    metrics: run.metrics,
                                    fallback_spans: Vec::new(),
                                    ink: None,
                                    ink_glyphs: Vec::new(),
                                });
                                let x = frag.rect.x + (frag.rect.width - gw * sc).max(0.0) / 2.0;
                                items.push(DisplayItem::Text {
                                    origin: (x, y),
                                    run: solo,
                                    font: font_request(&frag.style),
                                    color: ink,
                                    size_px: fs,
                                });
                                if sc < 1.0 {
                                    items.push(DisplayItem::PopTransform);
                                }
                                y += fs;
                                gi = end;
                                continue;
                            }
                        }
                        let g = &run.glyphs[gi];
                        gi += 1;
                        let cjk = frag
                            .text
                            .get(g.cluster as usize..)
                            .and_then(|s| s.chars().next())
                            .is_some_and(|ch| {
                                matches!(ch as u32,
                                    0x3000..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
                                    | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF)
                            });
                        let rot = orientation == 2 && !cjk;
                        if rot {
                            let ox = frag.rect.x + off.0;
                            items.push(DisplayItem::PushTransform(tiny_skia::Transform::from_row(
                                0.0,
                                1.0,
                                -1.0,
                                0.0,
                                ox + y,
                                y - ox,
                            )));
                        }
                        let solo = std::sync::Arc::new(ShapedRun {
                            glyphs: vec![*g],
                            advance: g.advance,
                            metrics: run.metrics,
                            fallback_spans: Vec::new(),
                            ink: None,
                            ink_glyphs: Vec::new(),
                        });
                        let x = frag.rect.x + (frag.rect.width - g.advance).max(0.0) / 2.0;
                        items.push(DisplayItem::Text {
                            origin: (x, y),
                            run: solo,
                            font: font_request(&frag.style),
                            color: ink,
                            size_px: fs,
                        });
                        if rot {
                            items.push(DisplayItem::PopTransform);
                        }
                        y += fs;
                    }
                } else {
                    items.push(DisplayItem::Text {
                        origin: (frag.rect.x + off.0, base_y),
                        run: run.clone(),
                        font: font_request(&frag.style),
                        color: ink,
                        size_px: fs,
                    });
                }
                if sideways_lr || sideways_rl {
                    items.push(DisplayItem::PopTransform);
                }
                // R-186 — `text-emphasis` : marque par
                // glyphe ; style+position lus sur le
                // fragment (shorthand non expansé —
                // `text-emphasis` seul suffit).
                if let Some((kind, ecolor, over)) = emphasis_kind(&frag.style, ink) {
                    items.push(DisplayItem::Emphasis {
                        origin: (frag.rect.x + off.0, base_y),
                        run: run.clone(),
                        size_px: fs,
                        kind,
                        color: ecolor,
                        over,
                    });
                }
                for d in &frag.deco {
                    if d.lines & explorer_layout::inline::DECO_LINE_THROUGH != 0 {
                        let dth = if d.thick_px > 0.0 { d.thick_px } else { th };
                        for rect in
                            deco_rect(d, base_y - fs * 0.3 - dth / 2.0, dth, d.skip_ink >= 2)
                        {
                            items.push(DisplayItem::DecoRect {
                                rect,
                                color: d.color.unwrap_or(ink),
                                style: d.style,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Décorations d'une pièce d'élément inline (R-22) : fond et
/// bordure sur `rect` — union des fragments de l'élément sur
/// la ligne — étendue de son padding et des épaisseurs de
/// bordure.
///
/// Approximation « `box-decoration-break: slice` » documentée :
/// une pièce par élément et par ligne, padding aux bords de
/// la pièce ; et `em`/`rem` ≈ font-size de l'élément (pas de
/// contexte racine ici).
fn paint_inline_deco(
    st: &explorer_css::ComputedStyle,
    rect: Rect,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
) {
    let fs = prop_px(st, "font-size", 16.0);
    #[allow(clippy::cast_possible_truncation)]
    let pad = |name: &str| -> f32 {
        let Some(l) = parse_length(st.get_or(name, "")) else {
            return 0.0;
        };
        match l.unit.px_factor() {
            Some(f) => (l.value * f) as f32,
            None => l.value as f32 * fs,
        }
    };
    let l = pad("padding-left") + border_side_w(st, "left");
    let r = pad("padding-right") + border_side_w(st, "right");
    let t = pad("padding-top") + border_side_w(st, "top");
    let b = pad("padding-bottom") + border_side_w(st, "bottom");
    let outer = shifted(
        Rect {
            x: rect.x - l,
            y: rect.y - t,
            width: rect.width + l + r,
            height: rect.height + t + b,
        },
        off,
    );
    if let Some(c) = st.color("background-color") {
        if c.a > 0 {
            items.push(DisplayItem::FillRect {
                rect: outer,
                color: c,
                radii: [0.0; 4],
                corners: [1.0; 4],
            });
        }
    }
    // `background-image` sur élément inline : hors portée
    // (rare — la plupart des fonds inline sont des couleurs).
    let widths = [
        border_side_w(st, "top"),
        border_side_w(st, "right"),
        border_side_w(st, "bottom"),
        border_side_w(st, "left"),
    ];
    if widths.iter().any(|v| *v > 0.0) {
        items.push(DisplayItem::Border {
            rect: outer,
            widths,
            colors: [
                border_color_of(st, "border-top-color"),
                border_color_of(st, "border-right-color"),
                border_color_of(st, "border-bottom-color"),
                border_color_of(st, "border-left-color"),
            ],
            styles: [
                border_style(st.get("border-top-style")),
                border_style(st.get("border-right-style")),
                border_style(st.get("border-bottom-style")),
                border_style(st.get("border-left-style")),
            ],
            radii: corner_radii(st, &outer),
            corners: corner_ks(st),
        });
    }
}

// `sticky_offset` vit dans `explorer_layout::hit` — source
// unique partagée par la display list et le hit-test (B-40 :
// un sticky épinglé est PEINT décalé par `PushTransform` ;
// le hit-test applique le même décalage à la cible).
use explorer_layout::sticky_offset;

/// `FontRequest` dérivé du style — **délègue à
/// `request_of_family`** (R-81) : une seule implémentation
/// pour mesure et peinture (avant : repli `"sans-serif"` en
/// dur + `bolder`→800/`lighter`→300 là où la mesure rend
/// 700/400 — corps `<p>` nu peint sans sur une mesure serif,
/// issue #9 4ᵉ défaut). `PaintCx` ne porte pas le `BlockCtx` :
/// le repli partagé est la constante
/// [`explorer_layout::font::DEFAULT_FAMILY`] — surcharger
/// `BlockCtx::default_family` exigerait de la propager jusque
/// là (journal `interface-equipes`).
fn font_request(st: &explorer_css::ComputedStyle) -> explorer_layout::font::FontRequest {
    explorer_layout::font::request_of_family(st, explorer_layout::font::DEFAULT_FAMILY)
}

/// `transform` d'une boîte → matrice tiny-skia (origine
/// incluse) — délègue à
/// [`explorer_layout::transform::box_transform`], source
/// unique partagée avec le hit-test (qui applique la
/// matrice inverse au point sondé — vol : zones cliquables
/// décalées sur éléments transformés).
fn box_transform(bx: &LayoutBox, rect: &Rect) -> Option<tiny_skia::Transform> {
    explorer_layout::transform::box_transform(bx, rect)
        .map(|m| tiny_skia::Transform::from_row(m.a, m.b, m.c, m.d, m.e, m.f))
}

/// Peint une boîte et ses descendants dans l'ordre borné.
/// La boîte ferme la portée de report des inlines : racine
/// (gérée à l'entrée), positionnée, flottante ou atome en
/// ligne (remplacée, `inline-block` à enfants blocs) — leurs
/// descendants en ligne restent peints à l'intérieur
/// (App. E : un flottant peint tout son sous-arbre à
/// l'étape 4, les inlines d'un contexte à l'étape 5).
fn inline_scope(bx: &LayoutBox) -> bool {
    // Tout conteneur de niveau bloc ferme la portée : ses
    // descendants en ligne se peignent dans SA phase inline
    // (App. E — récursion par élément, ordre document) — pas
    // à la fin du contexte de pilement, sinon leur débord
    // couvrirait la bordure des boîtes suivantes (wpt 155).
    !bx.kind.is_inline_level()
        || bx.position != Position::Static
        || bx.float != Float::None
        || bx.intrinsic.is_some()
        || bx.children.iter().any(|c| !c.kind.is_inline_level())
}

/// Enfant porteur de contenu en ligne — peint à l'étape 5 de
/// son contexte de pilement, pas à sa position d'arbre :
/// blocs anonymes (leurs lignes), boîtes `Inline`, `Text`.
fn inline_carrier(c: &LayoutBox) -> bool {
    c.float == Float::None
        && matches!(
            c.kind,
            BoxKind::AnonymousBlock | BoxKind::Inline | BoxKind::Text
        )
}

/// Peint `bx` avec sa propre portée de report — utilisé pour
/// les flottants, les positionnés et les porteurs inline
/// drainés : leurs descendants en ligne se peignent dedans.
/// Les positionnés `z ≥ 0` passent par `pos_defer` (file du
/// contexte d'empilement englobant) sauf si `bx` en est un.
fn paint_scoped<'a>(
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    fixed_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    let mut defer = Vec::new();
    paint_box(bx, items, off, cx, &mut defer, pos_defer, fixed_defer);
    let mut i = 0;
    while i < defer.len() {
        let (p, c) = defer[i].clone();
        i += 1;
        emit_scoped_at(p, c, items, off, cx, pos_defer, fixed_defer);
    }
}

/// Émet `bx` en réutilisant sa plage précédente si le
/// sous-arbre est ré-utilisable — sinon descente normale.
/// `idx` = son index dans `parent.children` (clé de span).
#[allow(clippy::too_many_arguments)]
fn emit_child<'a>(
    idx: usize,
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    fixed_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    cx.path.push(u16::try_from(idx).unwrap_or(u16::MAX));
    if let Some((d, s, sig)) = reuse_delta(bx, off, cx) {
        copy_span(bx, s, d, sig, items, off, cx);
        repush_escaped(bx, &s.escaped, cx, pos_defer, fixed_defer, Some(defer));
    } else {
        let start = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let base = defer.len();
        let pbase = pos_defer.len();
        let fbase = fixed_defer.len();
        explorer_common::deep(|| {
            paint_box(bx, items, off, cx, defer, pos_defer, fixed_defer);
        });
        let end = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let sig = box_sig(bx);
        let mut esc = escaped_paths(defer, base, &cx.path, cx).into_vec();
        esc.extend(escaped_paths(pos_defer, pbase, &cx.path, cx).into_vec());
        esc.extend(escaped_paths(fixed_defer, fbase, &cx.path, cx).into_vec());
        cx.record_span(
            bx,
            start,
            end,
            effective_off(bx, off),
            sig,
            esc.into_boxed_slice(),
        );
    }
    cx.path.pop();
}

/// Variante pour les enfants peints hors-flux (flottants,
/// positionnés, porteurs drainés) — portée propre.
#[allow(clippy::too_many_arguments)]
/// R-293 — marge d'encre hors `rect` pour la culle de bande
/// de [`emit_scoped`] : `box-shadow`/`text-shadow` peignent
/// au-delà de la boîte ; la marge couvre |offset| + blur +
/// spread de chaque couche externe (majoration — jamais de
/// sous-estimation : un sous-arbre sauté ne doit pouvoir
/// déposer aucune encre dans la bande).
fn ink_margin(bx: &LayoutBox) -> f32 {
    let mut m = 0.0_f32;
    for prop in ["box-shadow", "text-shadow"] {
        let Some(v) = bx.style.get(prop) else {
            continue;
        };
        if v.trim().eq_ignore_ascii_case("none") {
            continue;
        }
        for layer in bg_layers(v) {
            // Même grammaire de couches `x y blur spread`
            // (`text-shadow` ignore spread/inset — le parseur
            // commun les laisse à 0/false).
            if let Some((off, blur, spread, _c, inset)) =
                parse_box_shadow_layer(&bx.style, layer.trim())
            {
                if !inset {
                    m = m.max(off.0.abs() + off.1.abs() + blur + spread.max(0.0));
                }
            }
        }
    }
    m
}

fn emit_scoped<'a>(
    idx: usize,
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    fixed_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    cx.path.push(u16::try_from(idx).unwrap_or(u16::MAX));
    // R-293 — culle de bande, re-passes seulement (`inc` :
    // une liste précédente existe — la toute première passe
    // émet tout). Un sous-arbre prouvé contenu dans son
    // `rect` + marge d'encre (`!may_escape`) et entièrement
    // hors viewport ne produit aucun item : la descente est
    // sautée entière. `may_escape` couvre les reports
    // `pos_defer`/`fixed_defer`, transforms, flottants et
    // débords `overflow:visible` — ce qui est sauté ne peut
    // peindre nulle part ailleurs (§2.5 du plan perf).
    if cx.inc.is_some() && !bx.may_escape {
        let m = ink_margin(bx);
        let l = bx.rect.x + off.0;
        let t = bx.rect.y + off.1;
        if l - m >= cx.vp.0
            || t - m >= cx.vp.1
            || l + bx.rect.width + m <= 0.0
            || t + bx.rect.height + m <= 0.0
        {
            cx.n_culled += 1;
            cx.path.pop();
            return;
        }
    }
    // R-134 : translation sticky enregistrée par le parent à
    // l'enfilement — entoure l'émission quel que soit le chemin
    // d'émission (phase z<0, flottant, inline reporté).
    let delta = cx.sticky_deltas.remove(&(std::ptr::from_ref(bx) as usize));
    if let Some(d) = delta {
        items.push(DisplayItem::PushTransform(
            tiny_skia::Transform::from_translate(d.0, d.1),
        ));
    }
    if let Some((d, s, sig)) = reuse_delta(bx, off, cx) {
        let t = std::time::Instant::now();
        copy_span(bx, s, d, sig, items, off, cx);
        repush_escaped(bx, &s.escaped, cx, pos_defer, fixed_defer, None);
        cx.us_copy += u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX);
        cx.n_reuse += 1;
    } else {
        cx.n_paint += 1;
        let start = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let pbase = pos_defer.len();
        let fbase = fixed_defer.len();
        explorer_common::deep(|| {
            paint_scoped(bx, items, off, cx, pos_defer, fixed_defer);
        });
        let end = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let sig = box_sig(bx);
        // §B-26 : les porteurs poussés pendant cette fenêtre
        // (positionnés/`fixed` descendants) doivent survivre à
        // la copie — sinon un ré-usage perd leurs items
        // (under-paint incrémental). Les inlines restent
        // internes à `paint_scoped` : couverts par la plage.
        let t = std::time::Instant::now();
        let mut esc = escaped_paths(pos_defer, pbase, &cx.path, cx).into_vec();
        esc.extend(escaped_paths(fixed_defer, fbase, &cx.path, cx).into_vec());
        cx.record_span(
            bx,
            start,
            end,
            effective_off(bx, off),
            sig,
            esc.into_boxed_slice(),
        );
        cx.us_book += u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX);
    }
    if delta.is_some() {
        items.push(DisplayItem::PopTransform);
    }
    cx.path.pop();
}

/// Re-pousse les porteurs `escaped` d'un span copié — chemins
/// relatifs à `bx` → absolus ; le drain englobant ré-émettra
/// chaque cible via son propre span. La file se re-déduit :
/// `fixed` → `fixed_defer`, positionnés `z ≥ 0` → `pos_defer`,
/// autre → `else_q` (le `defer` de portée quand il existe —
/// sinon `pos_defer` : une boîte mutée de classe sous nos
/// pieds s'émet scoped plutôt que de disparaître).
fn repush_escaped<'a>(
    bx: &'a LayoutBox,
    escaped: &[EscapedDefer],
    cx: &mut PaintCx<'a>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    fixed_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    mut else_q: Option<&mut Vec<(Vec<u16>, &'a LayoutBox)>>,
) {
    for (rel, clips) in escaped {
        let Some(target) = resolve_rel(bx, rel) else {
            continue;
        };
        let mut p = cx.path.clone();
        p.extend(rel.iter().copied());
        // R-212 — l'instantané de clips pris lors de la
        // première émission est restauré : le span copié ne
        // retraverse pas les clippers internes.
        cx.defer_clips
            .insert(p.clone().into_boxed_slice(), clips.clone());
        if target.position == Position::Fixed {
            push_z(fixed_defer, 0, p, target);
        } else if !matches!(target.position, Position::Static | Position::Sticky)
            && target.z_index >= 0
        {
            push_z(pos_defer, 0, p, target);
        } else if let Some(d) = &mut else_q {
            d.push((p, target));
        } else {
            push_z(pos_defer, 0, p, target);
        }
    }
}

/// Variante pour les porteurs inline reportés — leur vrai
/// chemin (captured à l'enfilement, dans le parent direct)
/// remplace le chemin courant le temps de l'émission, sinon
/// leur span s'enregistrerait sous le chemin de l'ancêtre
/// drainant et pourrait entrer en collision avec un vrai
/// enfant de celui-ci.
/// Borne §B-26 : nombre maximal d'émissions d'un même chemin
/// par passe. Un porteur reporté doit être peint une fois ;
/// un cycle de report (une file qui se re-remplit de la même
/// cible) figeait le processus en croissance mémoire
/// monotone (meteofrance `[V5-main]` vol 65 — wedge entre
/// `apply: layout` et `apply: repaint`, +29 Mo/s).
const MAX_EMIT_PER_PATH: u32 = 4;

/// Plafond dur d'émissions reportées par passe (§B-26) —
/// très au-dessus de tout site réel (un porteur est émis
/// une fois ; quelques milliers suffisent aux plus grosses
/// pages) : le dépasser = une file qui se régénère.
const MAX_EMITS_TOTAL: u32 = 100_000;

fn emit_scoped_at<'a>(
    path: Vec<u16>,
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    fixed_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    cx.emits_total += 1;
    if cx.emits_total > MAX_EMITS_TOTAL {
        cx.dropped += 1;
        if cx.first_cycle_path.is_none() {
            cx.first_cycle_path = Some(path);
        }
        return;
    }
    // §B-26 : un chemin déjà émis `MAX_EMIT_PER_PATH` fois =
    // cycle de report — on saute plutôt que de nourrir la
    // file à l'infini (le contenu est dégradé, le processus
    // survit).
    let n = cx
        .emit_per_path
        .entry(path.clone().into_boxed_slice())
        .or_insert(0);
    *n += 1;
    if *n > MAX_EMIT_PER_PATH {
        cx.dropped += 1;
        if cx.first_cycle_path.is_none() {
            cx.first_cycle_path = Some(path);
        }
        return;
    }
    cx.stat(|s| s.deferred_emitted += 1);
    // R-212 — instantané des clips `overflow` applicables
    // (§11.1.1 : in-flow = tous ; absolute = bornés à son CB ;
    // fixed = aucun, jamais enregistrés) — ré-émis autour de la
    // peinture : sinon le report fuit le clipper ancêtre (nav de
    // mesure vimeo `h-0 overflow:hidden` affichée, vol 772).
    let clips = cx
        .defer_clips
        .get(path.as_slice())
        .cloned()
        .unwrap_or_default();
    for &(r, radii) in &*clips {
        if radii.iter().any(|v| *v > 0.0) {
            items.push(DisplayItem::PushClipRounded { rect: r, radii });
        } else {
            items.push(DisplayItem::PushClip(r));
        }
    }
    cx.clip_stack.extend_from_slice(&clips);
    let saved = std::mem::replace(&mut cx.path, path);
    // R-134 : translation sticky — même discipline que
    // `emit_scoped` (file reportée, drain en contexte
    // d'empilement quelconque).
    let delta = cx.sticky_deltas.remove(&(std::ptr::from_ref(bx) as usize));
    if let Some(d) = delta {
        items.push(DisplayItem::PushTransform(
            tiny_skia::Transform::from_translate(d.0, d.1),
        ));
    }
    if let Some((d, s, sig)) = reuse_delta(bx, off, cx) {
        let t = std::time::Instant::now();
        copy_span(bx, s, d, sig, items, off, cx);
        repush_escaped(bx, &s.escaped, cx, pos_defer, fixed_defer, None);
        cx.us_copy += u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX);
        cx.n_reuse += 1;
    } else {
        cx.n_paint += 1;
        let start = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let pbase = pos_defer.len();
        let fbase = fixed_defer.len();
        explorer_common::deep(|| {
            paint_scoped(bx, items, off, cx, pos_defer, fixed_defer);
        });
        let end = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let sig = box_sig(bx);
        // §B-26 : porteurs poussés pendant la fenêtre —
        // même bookkeeping que `emit_scoped` (sans `defer`,
        // portée interne à `paint_scoped`).
        let t = std::time::Instant::now();
        let mut esc = escaped_paths(pos_defer, pbase, &cx.path, cx).into_vec();
        esc.extend(escaped_paths(fixed_defer, fbase, &cx.path, cx).into_vec());
        cx.record_span(
            bx,
            start,
            end,
            effective_off(bx, off),
            sig,
            esc.into_boxed_slice(),
        );
        cx.us_book += u64::try_from(t.elapsed().as_micros()).unwrap_or(u64::MAX);
    }
    if delta.is_some() {
        items.push(DisplayItem::PopTransform);
    }
    cx.clip_stack.truncate(cx.clip_stack.len() - clips.len());
    for _ in 0..clips.len() {
        items.push(DisplayItem::PopClip);
    }
    cx.path = saved;
}

/// `off` réellement appliqué aux items de `bx` —
/// `position: fixed` ré-initialise l'offset (ses coords
/// sont viewport). `paint_box` fait de même : `reuse_delta`
/// et `copy_span` doivent comparer/enregistrer sur cette
/// base, sinon un fixe se voit appliquer le scroll du parent.
fn effective_off(bx: &LayoutBox, off: (f32, f32)) -> (f32, f32) {
    if bx.position == Position::Fixed {
        (0.0, 0.0)
    } else {
        off
    }
}

/// Δ de translation si le sous-arbre `bx` est ré-utilisable
/// en entier — `Some((d, span, sig))`, `None` = ré-émission
/// requise. **Invariant** : la copie plate d'une plage n'est
/// valide que si tous les items bougent du même Δ — chaque
/// descendant doit retourner le même Δ (un `fixed` interne
/// donne Δ=0 ≠ Δ du parent → le parent descend ; un sticky
/// est couvert par le flag `F_STICKY_KID` du parent).
fn reuse_delta<'a>(
    bx: &'a LayoutBox,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
) -> Option<((f32, f32), &'a SpanRec, u64)> {
    let inc = cx.inc?;
    let s = inc.spans.get(cx.path.as_slice())?;
    // Jamais de ré-usage sur média remplacé (bitmap tardif) ou
    // fond `attachment:fixed` (origine ancrée viewport — sa
    // translation divergerait de la géométrie).
    if s.flags & F_NO_REUSE != 0 {
        return None;
    }
    // Même objet LayoutBox (mutation in-place) : `lines`
    // (Vec réassigné par `relay`) et les enfants sont les
    // champs réalloués sur une boîte préservée — comparés ;
    // la signature est sautée (contrat consigné dans
    // `SpanRec::bx_ptr`, **vérifié** en debug ci-dessous).
    // `bx.node` n'est **pas** comparé : `remap_box_nodes`
    // le réécrit sur toute boîte préservée à chaque reparse
    // — et il n'est pas émis dans les items.
    // `kids_ptr` est le discriminant des remplacements
    // `*bx = fresh` : le slot garde son adresse mais le Vec
    // de `fresh` est alloué avant le drop de l'ancien —
    // disjoint. Une **feuille** (children + lines vides) n'a
    // que des ptrs dangling : rien ne la distingue d'une
    // remplacée au même slot → signature obligatoire.
    let leaf = bx.children.is_empty() && bx.lines.is_empty();
    let same_obj = !leaf
        && cx.same_tree
        && s.bx_ptr == std::ptr::from_ref(bx) as usize
        && s.lines_ptr == bx.lines.as_ptr() as usize
        && s.kids_ptr
            == if bx.children.is_empty() {
                usize::MAX
            } else {
                bx.children.as_ptr() as usize
            }
        && s.kids_len == u32::try_from(bx.children.len()).unwrap_or(u32::MAX);
    let sig = if same_obj {
        #[cfg(debug_assertions)]
        debug_assert_eq!(
            box_sig(bx),
            s.sig,
            "contrat R-37 violé : champ peint muté in-place              hors {{rect, node, lines, slot}} sur boîte préservée"
        );
        s.sig
    } else {
        let sig = box_sig(bx);
        if sig != s.sig {
            return None;
        }
        sig
    };
    if s.rect.width.to_bits() != bx.rect.width.to_bits()
        || s.rect.height.to_bits() != bx.rect.height.to_bits()
    {
        return None;
    }
    let eff = effective_off(bx, off);
    let d = (
        bx.rect.x - s.rect.x + eff.0 - s.off.0,
        bx.rect.y - s.rect.y + eff.1 - s.off.1,
    );
    // Enfant sticky → le wrap de décalage vit dans la plage
    // du parent et dépend du scroll non linéairement — toute
    // translation non nulle impose la ré-émission du parent.
    if s.flags & F_STICKY_KID != 0 && d != (0.0, 0.0) {
        return None;
    }
    for (i, c) in bx.children.iter().enumerate() {
        cx.path.push(u16::try_from(i).unwrap_or(u16::MAX));
        let cof = effective_off(c, off);
        let cd = explorer_common::deep(|| reuse_delta(c, cof, cx));
        cx.path.pop();
        // Translation uniforme exigée — un descendant qui
        // diverge (fixed, sale) interdit la copie plate.
        if cd.map(|(cd, _, _)| cd) != Some(d) {
            return None;
        }
    }
    Some((d, s, sig))
}

/// Copie la plage `s` en translatant chaque item de `d` et
/// enregistre le nouveau span de `bx`.
#[allow(clippy::too_many_arguments)]
fn copy_span(
    bx: &LayoutBox,
    s: &SpanRec,
    d: (f32, f32),
    sig: u64,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx,
) {
    let Some(inc) = cx.inc else { return };
    cx.stat(|s| s.spans_copied += 1);
    let start = u32::try_from(items.len()).unwrap_or(u32::MAX);
    for it in &inc.items[(s.start as usize)..(s.end as usize)] {
        items.push(translate_item(it, d));
    }
    let eff = effective_off(bx, off);
    // `escaped` est préservé : les chemins relatifs restent
    // valides dans le nouvel arbre (même sous-arbre).
    cx.record_span(
        bx,
        start,
        u32::try_from(items.len()).unwrap_or(u32::MAX),
        eff,
        sig,
        s.escaped.clone(),
    );
}

/// `bx` a-t-il du contenu remplacé/retardé — bitmap `<img>`,
/// `background-image`, svg, `<canvas>` (arrivent après le
/// paint, `Arc` nouvelle à chaque build) → ré-usage interdit.
fn is_media(bx: &LayoutBox, images: &crate::PaintImages) -> bool {
    bx.node.is_some_and(|n| {
        images.replaced.contains_key(&n)
            || images.svgs.contains_key(&n)
            || images.backgrounds.contains_key(&n)
            || images.background_layers.contains_key(&n)
            || images.canvases.contains_key(&n)
    }) || bx.style.get("background-attachment") == Some("fixed")
}

/// FNV-1a — fingerprint interne des spans (signature de
/// boîte + clés de `spans`). Pas cryptographique : le rôle
/// est de rejeter une égalité fausse, et le volume (~10⁴
/// spans par paint) exige ~1 ns/octet, pas `SipHash`.
#[derive(Clone)]
pub struct SigHasher(u64);

impl Default for SigHasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl SigHasher {
    #[inline]
    fn mix(&mut self, v: u64) {
        for b in v.to_ne_bytes() {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    #[inline]
    fn mix_f32(&mut self, v: f32) {
        self.mix(u64::from(v.to_bits()));
    }
}

impl std::hash::Hasher for SigHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

/// Table de correspondance boîte→plage — hasher rapide (les
/// clés sont des chemins courts `u16`, hashés à chaque
/// span produit *et* cherché).
pub type SpanMap =
    std::collections::HashMap<Box<[u16]>, SpanRec, std::hash::BuildHasherDefault<SigHasher>>;

/// Fingerprint des champs lus par l'émission hors géométrie —
/// deux boîtes de `sig` égale produisent des items égaux aux
/// mêmes positions (modulo `rect`/`off`, comparés à part).
/// Couvre kind/positions/overflow/z-index/checked/intrinsic/
/// texte/style/lignes (rects de fragments, contenu texte et
/// **identité** des runs shapés — le `Arc` refait à chaque
/// layout, donc conservateur : même texte re-shapé ≠ réusé).
fn box_sig(bx: &LayoutBox) -> u64 {
    use std::hash::Hasher;
    let mut h = SigHasher::default();
    // Discriminants d'enums → petit rang stable.
    h.mix(match bx.kind {
        BoxKind::Block => 0,
        BoxKind::ListItem => 1,
        BoxKind::Inline => 2,
        BoxKind::Text => 3,
        BoxKind::AnonymousBlock => 4,
    });
    h.mix(match bx.position {
        Position::Static => 0,
        Position::Relative => 1,
        Position::Absolute => 2,
        Position::Fixed => 3,
        Position::Sticky => 4,
    });
    h.mix(match bx.float {
        Float::None => 0,
        Float::Left => 1,
        Float::Right => 2,
    });
    h.mix(match bx.overflow {
        Overflow::Visible => 0,
        Overflow::Hidden | Overflow::Clip => 1,
        Overflow::Scroll => 2,
        Overflow::Auto => 3,
    });
    h.mix(match bx.overflow_x {
        Overflow::Visible => 0,
        Overflow::Hidden | Overflow::Clip => 1,
        Overflow::Scroll => 2,
        Overflow::Auto => 3,
    });
    h.mix(u64::from(u32::from_ne_bytes(bx.z_index.to_ne_bytes())));
    h.mix(u64::from(bx.control_checked));
    let (iw, ih) = bx.intrinsic.unwrap_or((0.0, 0.0));
    h.mix_f32(iw);
    h.mix_f32(ih);
    if let Some(t) = &bx.text {
        h.write(t.as_bytes());
    }
    h.mix(0x9e37);
    // Style : itération d'ordre instable → hash par paire,
    // pli commutatif (XOR) — ordre-insensible.
    let mut sh = 0u64;
    for (k, v) in bx.style.iter() {
        let mut ph = SigHasher::default();
        ph.write(k.as_bytes());
        ph.write(v.as_bytes());
        sh ^= ph.finish();
    }
    h.mix(sh);
    for line in &bx.lines {
        for f in &line.fragments {
            // La géométrie des fragments (rect, baseline,
            // marges, trail_w) est **dérivée** des entrées
            // signées (style + texte + taille du conteneur,
            // comparée à part) — et `translate_y` la décale
            // in-place sous `relayout_subtree` : l'inclure
            // violerait le contrat du fast-path. Elle est
            // couverte par `s.rect` + l'invariant Δ-uniforme.
            h.write(f.text.as_bytes());
            h.mix(u64::from(f.atomic));
            // Contenu du run (pas le ptr `Arc` — instable
            // entre passes) : une fonte chargée entre deux
            // builds change les glyphes → la signature doit
            // l'attraper. Même ordre de coût que le texte.
            if let Some(r) = &f.shaped {
                h.mix(r.glyphs.len() as u64);
                for g in &r.glyphs {
                    h.mix(u64::from(g.id));
                    h.mix_f32(g.advance);
                    h.mix_f32(g.offset_x);
                    h.mix_f32(g.offset_y);
                }
            }
            let mut fh = 0u64;
            for (k, v) in f.style.iter() {
                let mut ph = SigHasher::default();
                ph.write(k.as_bytes());
                ph.write(v.as_bytes());
                fh ^= ph.finish();
            }
            h.mix(fh);
        }
    }
    h.finish()
}

/// Item translaté de `d` — utilisé par les copies de plage
/// incrémentales. `d = (0,0)` → clone simple.
#[allow(clippy::too_many_lines)]
fn translate_item(it: &DisplayItem, d: (f32, f32)) -> DisplayItem {
    use DisplayItem as D;
    if d == (0.0, 0.0) {
        return it.clone();
    }
    match it {
        D::FillRect {
            rect,
            color,
            radii,
            corners,
        } => D::FillRect {
            rect: shifted(*rect, d),
            color: *color,
            radii: *radii,
            corners: *corners,
        },
        D::DecoRect { rect, color, style } => D::DecoRect {
            rect: shifted(*rect, d),
            color: *color,
            style: *style,
        },
        D::BoxShadow {
            rect,
            offset,
            blur,
            spread,
            color,
            radii,
        } => D::BoxShadow {
            rect: shifted(*rect, d),
            offset: *offset,
            blur: *blur,
            spread: *spread,
            color: *color,
            radii: *radii,
        },
        D::Border {
            rect,
            widths,
            colors,
            styles,
            radii,
            corners,
        } => D::Border {
            rect: shifted(*rect, d),
            widths: *widths,
            colors: *colors,
            styles: *styles,
            radii: *radii,
            corners: *corners,
        },
        D::Text {
            origin,
            run,
            font,
            color,
            size_px,
        } => D::Text {
            origin: (origin.0 + d.0, origin.1 + d.1),
            run: run.clone(),
            font: font.clone(),
            color: *color,
            size_px: *size_px,
        },
        D::Placeholder { rect, label } => D::Placeholder {
            rect: shifted(*rect, d),
            label: label.clone(),
        },
        D::Image { rect, img } => D::Image {
            rect: shifted(*rect, d),
            img: img.clone(),
        },
        D::BackgroundImage {
            rect,
            img,
            repeat,
            origin,
            blend,
        } => D::BackgroundImage {
            rect: shifted(*rect, d),
            img: img.clone(),
            repeat: *repeat,
            origin: (origin.0 + d.0, origin.1 + d.1),
            blend: *blend,
        },
        D::MaskedFill {
            rect,
            color,
            mask,
            dst,
            repeat,
            luminance,
        } => D::MaskedFill {
            rect: shifted(*rect, d),
            color: *color,
            mask: mask.clone(),
            dst: shifted(*dst, d),
            repeat: *repeat,
            luminance: *luminance,
        },
        D::BorderImage {
            rect,
            img,
            src_edges,
            dst_edges,
            fill_center,
            repeat,
        } => D::BorderImage {
            rect: shifted(*rect, d),
            img: img.clone(),
            src_edges: *src_edges,
            dst_edges: *dst_edges,
            fill_center: *fill_center,
            repeat: *repeat,
        },
        D::Marker { rect, kind, color } => D::Marker {
            rect: shifted(*rect, d),
            kind: *kind,
            color: *color,
        },
        D::MarkerText {
            origin,
            text,
            font,
            color,
            size_px,
        } => D::MarkerText {
            origin: (origin.0 + d.0, origin.1 + d.1),
            text: text.clone(),
            font: font.clone(),
            color: *color,
            size_px: *size_px,
        },
        D::PushClip(r) => D::PushClip(shifted(*r, d)),
        D::PushClipRounded { rect, radii } => D::PushClipRounded {
            rect: shifted(*rect, d),
            radii: *radii,
        },
        D::PushTransform(m) => {
            let mut m = *m;
            m.tx += d.0;
            m.ty += d.1;
            D::PushTransform(m)
        }
        D::PushClipPath { points } => D::PushClipPath {
            points: points.iter().map(|p| (p.0 + d.0, p.1 + d.1)).collect(),
        },
        D::PushMask { rect, layers } => D::PushMask {
            rect: shifted(*rect, d),
            layers: layers
                .iter()
                .map(|l| MaskTile {
                    mask: l.mask.clone(),
                    dst: shifted(l.dst, d),
                    repeat: l.repeat,
                    luminance: l.luminance,
                    composite: l.composite,
                })
                .collect(),
        },
        D::PopClip
        | D::PopTransform
        | D::PushFilter(_)
        | D::PopFilter
        | D::PushBlend(_)
        | D::PopBlend
        | D::PopMask => it.clone(),
        D::Emphasis {
            origin,
            run,
            size_px,
            kind,
            color,
            over,
        } => D::Emphasis {
            origin: (origin.0 + d.0, origin.1 + d.1),
            run: run.clone(),
            size_px: *size_px,
            kind: *kind,
            color: *color,
            over: *over,
        },
        D::BackdropFilter { rect, ops } => D::BackdropFilter {
            rect: shifted(*rect, d),
            ops: ops.clone(),
        },
    }
}

/// Élément remplacé : bitmap par-dessus bordure (contenu —
/// ordre CSS 2.1 App. E : fond → bordure → contenu).
/// Budget pixels d'un `<svg>` re-rasterisé à la taille
/// utilisée (R-133) — borne mémoire par image, indépendante
/// du contexte session (inaccessible ici).
const SVG_RESCALE_MAX_PX: u32 = 4_000_000;

fn paint_replaced(
    bx: &LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    images: &crate::PaintImages,
) {
    if let Some(img) = bx.node.and_then(|n| {
        images
            .replaced
            .get(&n)
            .or_else(|| images.svgs.get(&n))
            .cloned()
            .or_else(|| {
                // `<canvas>` : op-list rejouée vers un bitmap à
                // la taille intrinsèque de l'élément (attributs
                // width/height — défaut spec 300×150).
                images.canvases.get(&n).and_then(|ops| {
                    let (w, h) = bx.intrinsic.unwrap_or((300.0, 150.0));
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    crate::canvas::raster_ops(ops, w.max(0.0) as u32, h.max(0.0) as u32)
                        .map(std::sync::Arc::new)
                })
            })
    }) {
        // Le contenu remplacé se place dans la **content
        // box** — padding et bordures restent autour
        // (§10.3.2 — Acid2 : l'œil 96×24 étiré à 131 sinon,
        // couvrant padding 11/12 + bordure 12). `object-fit`
        // (R-34) positionne le bitmap dedans ; les modes qui
        // débordent (`cover`, `none` trop grand, position
        // décalée) sont clipés à la content box.
        let content = shifted(content_rect(bx), off);
        let (mut dst, mut clip) = object_fit_dst(bx, &img, content);
        // R-133 (§B-32/§B-36) : un `<svg>` inline est
        // pré-rasterisé à sa taille **naturelle** (collecte
        // pré-layout) — peint dans une boîte plus grande, le
        // bitmap s'étire en blocs (icône `viewBox` 24 peinte
        // 130 px = ×5,4 — logo regex101, wordmark indeed).
        // La source XML étant conservée (`svg_sources`), on
        // re-rasterise à la destination exacte quand
        // l'agrandissement est notable (> 1,25× — sous ce
        // seuil le coût usvg ne sert à rien). `none`/
        // `scale-down` peignent à la taille naturelle :
        // re-rasteriser gonflerait la taille peinte.
        #[allow(clippy::cast_precision_loss)] // bitmaps < 2^24 px par axe.
        if !matches!(
            bx.style.get("object-fit").map(str::trim),
            Some("none" | "scale-down")
        ) && (dst.width > img.width as f32 * 1.25 || dst.height > img.height as f32 * 1.25)
        {
            // `svg_sources` n'est peuplé que pour les racines
            // `<svg>` — sa présence suffit à identifier le cas.
            if let Some(src) = bx.node.and_then(|n| images.svg_sources.get(&n)) {
                if let Some(big) = crate::image::decode_svg_raster_sized(
                    src.as_bytes(),
                    SVG_RESCALE_MAX_PX,
                    (dst.width, dst.height),
                ) {
                    let img = std::sync::Arc::new(big);
                    (dst, clip) = object_fit_dst(bx, &img, content);
                    if clip {
                        items.push(DisplayItem::PushClip(content));
                    }
                    items.push(DisplayItem::Image { rect: dst, img });
                    if clip {
                        items.push(DisplayItem::PopClip);
                    }
                    return;
                }
            }
        }
        if clip {
            items.push(DisplayItem::PushClip(content));
        }
        items.push(DisplayItem::Image { rect: dst, img });
        if clip {
            items.push(DisplayItem::PopClip);
        }
    }
}

/// Destination `object-fit` (R-34, minimal) dans la content
/// box : `fill` (défaut) étire le bitmap à la boîte ;
/// `contain`/`scale-down`/`cover`/`none` préservent le ratio
/// ou la taille naturelle ; `object-position` règle
/// l'ancrage dans l'espace libre (50 % par axe = centré).
/// Renvoie `(rect, clip)` — `clip` vrai quand la destination
/// déborde la boîte (le peintre pose un `PushClip`).
fn object_fit_dst(bx: &LayoutBox, img: &crate::DecodedImage, content: Rect) -> (Rect, bool) {
    #[allow(clippy::cast_precision_loss)] // bitmaps < 2^24 px par axe.
    let (iw, ih) = (img.width as f32, img.height as f32);
    if iw <= 0.0 || ih <= 0.0 || content.width <= 0.0 || content.height <= 0.0 {
        return (content, false);
    }
    let (w, h) = match bx.style.get("object-fit").map_or("fill", |v| v.trim()) {
        "contain" => {
            let s = (content.width / iw).min(content.height / ih);
            (iw * s, ih * s)
        }
        "cover" => {
            let s = (content.width / iw).max(content.height / ih);
            (iw * s, ih * s)
        }
        "none" => (iw, ih),
        "scale-down" => {
            let s = (content.width / iw).min(content.height / ih).min(1.0);
            (iw * s, ih * s)
        }
        // `fill` et toute valeur inconnue : étirement pleine
        // boîte (comportement historique — tolérance CSS).
        _ => (content.width, content.height),
    };
    let (ox, oy) = object_position(&bx.style, content.width - w, content.height - h);
    let dst = Rect {
        x: content.x + ox,
        y: content.y + oy,
        width: w,
        height: h,
    };
    let clip = dst.x < content.x
        || dst.y < content.y
        || dst.x + dst.width > content.x + content.width
        || dst.y + dst.height > content.y + content.height;
    (dst, clip)
}

/// `object-position` → décalage `(x, y)` en px dans
/// l'espace libre de chaque axe : mots-clés
/// (`left`/`center`/`right` × `top`/`center`/`bottom`),
/// `<pourcentage>` (part de l'espace libre — 50 % centre,
/// défaut) et `<longueur>` px. Une valeur unique sans axe
/// explicite reçoit l'axe horizontal ; l'axe non couvert
/// reste centré (spec `<position>`).
fn object_position(st: &explorer_css::ComputedStyle, free_x: f32, free_y: f32) -> (f32, f32) {
    let (mut x, mut y) = (None, None);
    if let Some(v) = st.get("object-position") {
        for tok in v.split_whitespace().take(2) {
            match tok.to_ascii_lowercase().as_str() {
                "left" => x = Some(0.0),
                "right" => x = Some(free_x),
                "top" => y = Some(0.0),
                "bottom" => y = Some(free_y),
                "center" => {
                    if x.is_none() {
                        x = Some(free_x / 2.0);
                    } else {
                        y = Some(free_y / 2.0);
                    }
                }
                t => {
                    let off = t
                        .strip_suffix('%')
                        .and_then(|p| p.parse::<f32>().ok())
                        .map(|n| (n / 100.0, true))
                        .or_else(|| {
                            t.strip_suffix("px")
                                .and_then(|p| p.parse::<f32>().ok())
                                .map(|n| (n, false))
                        });
                    if let Some((n, frac)) = off {
                        if x.is_none() {
                            x = Some(if frac { free_x * n } else { n });
                        } else {
                            y = Some(if frac { free_y * n } else { n });
                        }
                    }
                }
            }
        }
    }
    (x.unwrap_or(free_x / 2.0), y.unwrap_or(free_y / 2.0))
}

/// La boîte crée-t-elle un contexte d'empilement au sens de
/// l'App. E ? Positionnée à `z-index` explicite, ou transformée
/// (CSS Transforms). La racine est traitée par l'appelant.
/// `auto` et `0` ne sont pas distingués par `z_index` (champ
/// aplati) — on lit le style calculé.
fn stacking_ctx(bx: &LayoutBox) -> bool {
    (bx.position != Position::Static && bx.style.get("z-index").is_some_and(|v| v.trim() != "auto"))
        || bx
            .style
            .get("transform")
            .is_some_and(|v| v.trim() != "none")
        // R-185 — inducteurs de contexte d'empilement (spec
        // css-compositing + will-change + contain) :
        // `mix-blend-mode`/`isolation` actifs, `will-change`
        // à valeur transformée, `contain:paint|strict|content`.
        || bx
            .style
            .get("mix-blend-mode")
            .is_some_and(|v| v.trim() != "normal")
        || bx.style.get("isolation").is_some_and(|v| v.trim() == "isolate")
        || bx
            .style
            .get("will-change")
            .is_some_and(|v| v.split(',').any(|t| matches!(t.trim(), "transform" | "opacity" | "filter")))
        || bx
            .style
            .get("contain")
            .is_some_and(|v| v.split(' ').any(|t| matches!(t, "paint" | "strict" | "content")))
        // R-198 — `clip-path` et `filter` non-`none` créent un
        // stacking context ET un containing block pour les
        // descendants abs/fixed (css-masking-1, filter-effects) :
        // leurs positionnés z ≥ 0 doivent drainer dans `my_pos`
        // — à l'intérieur du `PushClipPath`/`PushFilter` —
        // comme les descendants d'un ancêtre transformé.
        || bx
            .style
            .get("clip-path")
            .is_some_and(|v| v.trim() != "none")
        || bx
            .style
            .get("filter")
            .is_some_and(|v| v.trim() != "none")
        // `transform-style: preserve-3d` — propriété de
        // regroupement (css-transforms-2) : stacking context
        // même aplati (moteur planaire, scène 3D = résidu).
        || bx
            .style
            .get("transform-style")
            .is_some_and(|v| v.trim() == "preserve-3d")
}

/// Enfile un enfant positionné dans la file de son contexte
/// d'empilement — insertion triée par `z-index`, stable (les
/// égalités gardent l'ordre d'arbre — App. E étapes 8-9).
/// Insère `(path, bx)` trié par `z-index` (stable) — `lo` est la
/// frontière du préfixe déjà drainé : la position se calcule
/// parmi les entrées non lues seulement. Sans elle, une file en
/// cours de drain verrait une entrée fusionnée s'insérer sous le
/// curseur — jamais drainée — ou déplacer une entrée déjà émise
/// au-delà du curseur, qui refait surface sans fin (§B-26).
fn push_z<'a>(
    q: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    lo: usize,
    path: Vec<u16>,
    bx: &'a LayoutBox,
) {
    let z = bx.z_index;
    let at = lo + q[lo.min(q.len())..].partition_point(|(_, c)| c.z_index <= z);
    q.insert(at, (path, bx));
}

/// R-58 — clip géométrique `clip-path: inset(…)` /
/// `clip: rect(…)` sur la border box (boîte de référence par
/// défaut, spec CSS Masking). v1 : `circle()`/`ellipse()`/
/// `polygon()`, la clause `round <radius>`, `url()` et les
/// `<geometry-box>` explicites retombent sur « pas de clip »
/// (tolérance — jamais de clip faux). `clip` ne s'applique
/// qu'aux boîtes absolument positionnées (CSS2 §11.1.2 —
/// le champ `position` peut rester `Static` sur un inline
/// positionné → lecture au style calculé). Les deux
/// propriétés se combinent par intersection.
#[allow(clippy::cast_possible_truncation)]
fn clip_geo_rect(bx: &LayoutBox, r: Rect, vp: (f32, f32)) -> Option<Rect> {
    let fs = bx
        .style
        .length("font-size")
        .map_or(16.0, |l| l.value as f32);
    let px = |tok: &str, base: f32| -> Option<f32> {
        let t = tok.trim();
        if t.eq_ignore_ascii_case("auto") {
            return None;
        }
        if let Some(p) = t.strip_suffix('%') {
            return p
                .trim()
                .parse::<f64>()
                .ok()
                .map(|n| (n as f32) * base / 100.0);
        }
        let l = parse_length(t)?;
        Some(match l.unit {
            Unit::Em | Unit::Rem => l.value as f32 * fs,
            Unit::Ex | Unit::Ch => l.value as f32 * fs * 0.5,
            Unit::Vw => l.value as f32 * vp.0 / 100.0,
            Unit::Vh => l.value as f32 * vp.1 / 100.0,
            Unit::Vmin => l.value as f32 * vp.0.min(vp.1) / 100.0,
            Unit::Vmax => l.value as f32 * vp.0.max(vp.1) / 100.0,
            u => (l.value * u.px_factor()?) as f32,
        })
    };
    let mut out: Option<Rect> = None;
    let mut and = |c: Rect| {
        out = Some(match out {
            None => c,
            // Intersection : le plus restrictif des deux clips.
            Some(a) => Rect {
                x: a.x.max(c.x),
                y: a.y.max(c.y),
                width: (a.x + a.width).min(c.x + c.width) - a.x.max(c.x),
                height: (a.y + a.height).min(c.y + c.height) - a.y.max(c.y),
            },
        });
        if let Some(o) = out {
            if o.width < 0.0 || o.height < 0.0 {
                out = Some(Rect {
                    width: 0.0,
                    height: 0.0,
                    ..o
                });
            }
        }
    };
    if let Some(v) = bx.style.get("clip-path") {
        if let Some(r2) = clip_path_inset(v.trim(), r, &px) {
            and(r2);
        }
    }
    let pos = bx.style.get("position").unwrap_or("static");
    if matches!(pos, "absolute" | "fixed") {
        if let Some(v) = bx.style.get("clip") {
            if let Some(r2) = legacy_clip_rect(v.trim(), r, &px) {
                and(r2);
            }
        }
    }
    out
}

/// `inset(t r b l [round …])` — 1 à 4 retraits depuis les
/// bords (top/right/bottom/left, raccourci façon `margin`) ;
/// la clause `round` est acceptée mais ignorée en v1
/// (coins carrés). `None` si la forme n'est pas un `inset()`
/// ou si un côté est inclassable (tolérance : pas de clip).
fn clip_path_inset(
    raw: &str,
    bounds: Rect,
    to_px: &dyn Fn(&str, f32) -> Option<f32>,
) -> Option<Rect> {
    let inner = raw
        .strip_prefix("inset")?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    // Coupe la clause `round` éventuelle — les coins restent
    // carrés en v1.
    let inner = inner.split_once("round").map_or(inner, |(a, _)| a).trim();
    let toks: Vec<&str> = inner.split_whitespace().collect();
    if toks.is_empty() || toks.len() > 4 {
        return None;
    }
    let top = to_px(toks[0], bounds.height)?;
    let right = to_px(toks.get(1).unwrap_or(&toks[0]), bounds.width)?;
    let bottom = to_px(toks.get(2).unwrap_or(&toks[0]), bounds.height)?;
    let left = to_px(
        toks.get(3).unwrap_or(toks.get(1).unwrap_or(&toks[0])),
        bounds.width,
    )?;
    Some(Rect {
        x: bounds.x + left,
        y: bounds.y + top,
        width: (bounds.width - left - right).max(0.0),
        height: (bounds.height - top - bottom).max(0.0),
    })
}

/// `clip: rect(t, r, b, l)` (CSS2 §11.1.2 — virgules
/// facultatives) : bords mesurés depuis le coin haut-gauche
/// — `top`/`bottom` depuis le haut, `left`/`right` depuis la
/// gauche ; `auto` = bord de la boîte. `None` si la forme
/// n'est pas `rect()` ou le compte ≠ 4.
fn legacy_clip_rect(v: &str, r: Rect, px: &dyn Fn(&str, f32) -> Option<f32>) -> Option<Rect> {
    let inner = v
        .strip_prefix("rect")?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let toks: Vec<&str> = inner.split([',', ' ']).filter(|s| !s.is_empty()).collect();
    if toks.len() != 4 {
        return None;
    }
    // `auto` = le bord correspondant de la boîte.
    let side = |tok: &str, base: f32, dft: f32| -> Option<f32> {
        if tok.trim().eq_ignore_ascii_case("auto") {
            Some(dft)
        } else {
            px(tok, base)
        }
    };
    let top = side(toks[0], r.height, 0.0)?;
    let right = side(toks[1], r.width, r.width)?;
    let bottom = side(toks[2], r.height, r.height)?;
    let left = side(toks[3], r.width, 0.0)?;
    let x = r.x + left;
    let y = r.y + top;
    Some(Rect {
        x,
        y,
        width: (r.x + right - x).max(0.0),
        height: (r.y + bottom - y).max(0.0),
    })
}

/// R-188 — `backface-visibility:hidden` + transform 3D
/// dont le déterminant projeté est négatif (dos tourné) →
/// la boîte ne se peint pas. Seule la transform **propre**
/// compte : sans `preserve-3d` (non implémenté), les enfants
/// s'aplatissent dans le plan du parent — leur « dos » ne
/// dépend alors que de leur propre matrice (css-transforms-2
/// — le cas hérité d'ancêtre n'existe qu'en preserve-3d).
fn backface_hidden(bx: &LayoutBox) -> bool {
    bx.style
        .get("backface-visibility")
        .is_some_and(|v| v.trim() == "hidden")
        && box_transform(bx, &bx.rect).is_some_and(|m| m.sx * m.sy - m.kx * m.ky < 0.0)
}

fn paint_box<'a>(
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    fixed_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    // R-188 — dos tourné : évalué AVANT tout push — un
    // `return` plus tard laisserait des `PushClip`/
    // `PushFilter`/`PushBlend` orphelins sur la pile raster.
    if backface_hidden(bx) {
        let p = cx.path.clone();
        cx.stat(|s| {
            s.drop_backface += 1;
            s.backface_path.get_or_insert(p);
        });
        return;
    }
    // R-252 — `mask-image` de groupe (css-masking-1) : quand
    // la boîte peint plus que son `background-color` (enfants
    // ou lignes), l'alpha du mask s'applique à TOUT le
    // sous-arbre — bracket **outermost** (ordre spec :
    // filter → clip → mask). Les feuilles « icône pure »
    // gardent `MaskedFill` (pas de buffer offscreen). Limites
    // documentées — communes aux autres brackets : les
    // descendants reportés hors racine (`pos_defer` au
    // contexte ancêtre, `fixed_defer`) échappent au groupe.
    let group_mask = if bx.children.is_empty() && bx.lines.is_empty() {
        Vec::new()
    } else {
        resolve_mask(bx, &shifted(bx.rect, off), cx.images)
    };
    if let Some((first, layers)) = group_mask.split_first() {
        items.push(DisplayItem::PushMask {
            rect: first.clip,
            layers: layers
                .iter()
                .map(|l| MaskTile {
                    mask: l.mask.clone(),
                    dst: l.dst,
                    repeat: l.repeat,
                    luminance: l.luminance,
                    composite: l.composite,
                })
                .collect(),
        });
    }
    // R-M5 — `filter` encadre TOUTE la sous-arbre de la
    // boîte (contenu + descendants) : bracket hors-écran
    // `PushFilter`…`PopFilter` ; la récursion des enfants
    // bracket le leur à leur tour (imbrication correct).
    // `drop-shadow()`/`url()` non portés.
    let fops = parse_filter(&bx.style);
    let filtered = !fops.is_empty();
    if filtered {
        items.push(DisplayItem::PushFilter(fops));
    }
    // R-185 — `mix-blend-mode` : le sous-arbre entier se
    // fond avec le fond accumulé ; `isolation:isolate` →
    // buffer isolant (tag 0) qui borne les fondus internes.
    let blend = blend_tag(bx.style.get("mix-blend-mode"));
    let isolated = bx
        .style
        .get("isolation")
        .is_some_and(|v| v.trim() == "isolate");
    if blend > 0 || isolated {
        items.push(DisplayItem::PushBlend(blend));
    }
    // R-185 — `contain:paint|strict|content` : le contenu
    // de la boîte ne peint pas hors de sa border box.
    let paint_contained = bx.style.get("contain").is_some_and(|v| {
        v.split(' ')
            .any(|t| matches!(t, "paint" | "strict" | "content"))
    });
    if paint_contained {
        items.push(DisplayItem::PushClip(shifted(bx.rect, off)));
    }
    // R-187 — `clip-path` : la boîte (fond+contenu) ne se
    // peint que dans la géométrie — même pile `clips`.
    // `off` aligne le clip sur la position peinte
    // (rejeu décalé), comme les autres clips.
    let clip_item = clip_path_of(&bx.style, &shifted(bx.rect, off));
    if let Some(it) = &clip_item {
        items.push(it.clone());
    }
    paint_box_impl(bx, items, off, cx, defer, pos_defer, fixed_defer);
    if clip_item.is_some() {
        items.push(DisplayItem::PopClip);
    }
    if paint_contained {
        items.push(DisplayItem::PopClip);
    }
    if blend > 0 || isolated {
        items.push(DisplayItem::PopBlend);
    }
    if filtered {
        items.push(DisplayItem::PopFilter);
    }
    if !group_mask.is_empty() {
        items.push(DisplayItem::PopMask);
    }
}

/// Corps de [`paint_box`] — le wrapper pose le bracket
/// `filter` (R-M5) autour de tout le sous-arbre.
#[allow(clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
fn paint_box_impl<'a>(
    bx: &'a LayoutBox,
    items: &mut Vec<DisplayItem>,
    off: (f32, f32),
    cx: &mut PaintCx<'a>,
    defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    pos_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
    fixed_defer: &mut Vec<(Vec<u16>, &'a LayoutBox)>,
) {
    cx.stat(|s| s.boxes += 1);
    // `position: fixed` est ancrée au viewport : son sous-arbre
    // ignore le scroll.
    let off = if bx.position == Position::Fixed {
        (0.0, 0.0)
    } else {
        off
    };

    // `opacity: 0` (§3.2, groupe alpha nul) : tout le
    // sous-arbre est invisible — court-circuit AVANT la
    // descente (transform/clip/enfants n'émettent rien de
    // visible de toute façon). Contrairement à
    // `visibility:hidden` qui permet la réactivation par un
    // descendant, `opacity` est héritée multiplicativement
    // : 0 ne se relève jamais. Vol 2026-10-26 : les modales
    // `opacity-0` (bookshop « Our mission », overlay ecma)
    // peintaient en plein. Les opacités partielles
    // (0 < o < 1) restent non appliquées — MVP : le groupe
    // alpha complet est reporté.
    if bx
        .style
        .get("opacity")
        .is_some_and(|v| v.trim().parse::<f32>().is_ok_and(|o| o <= 0.0))
    {
        let p = cx.path.clone();
        cx.stat(|s| {
            s.drop_opacity0 += 1;
            s.opacity0_path.get_or_insert(p);
        });
        return;
    }

    // `transform` : matrice poussée avant tout item du
    // sous-arbre, retiré après (imbrication additive).
    // R-272 — `offset-path` : transform pré-calculée par le
    // parent (bloc conteneur inconnu ici) — composée AVANT le
    // `transform` propre (motion path d'abord, spec §5).
    let off_t = cx.offset_tf.remove(&(std::ptr::from_ref(bx) as usize));
    let tf = match (box_transform(bx, &shifted(bx.rect, off)), off_t) {
        (Some(css), Some(off)) => Some(
            // `transform` propre d'abord, motion ensuite :
            // css.post_concat(off) = off ∘ css.
            tiny_skia::Transform::from_row(off.a, off.b, off.c, off.d, off.e, off.f)
                .post_concat(css),
        ),
        (None, Some(off)) => Some(tiny_skia::Transform::from_row(
            off.a, off.b, off.c, off.d, off.e, off.f,
        )),
        (t, None) => t,
    };
    if let Some(m) = tf {
        items.push(DisplayItem::PushTransform(m));
    }
    // R-58 — `clip-path`/`clip` : clip géométrique sur la
    // border box — il borne TOUT le rendu de la boîte (fond,
    // bordures, lignes, enfants) contrairement au clip
    // `overflow` qui n'englobe que le contenu. Émis dans la
    // portée transform : le raster transforme le rect comme
    // le clip `overflow`. Limite v1 : les descendants
    // positionnés reportés au contexte ancêtre en sortent.
    let geo_clip = clip_geo_rect(bx, shifted(bx.rect, off), cx.vp);
    if let Some(gc) = geo_clip {
        items.push(DisplayItem::PushClip(gc));
    }
    // `visibility` (§11.2) : `hidden`/`collapse` interdit
    // l'émission PROPRE mais la boîte garde sa place en
    // layout et la descente continue — un descendant
    // `visibility:visible` explicite se repeint (son style
    // calculé porte déjà l'héritage → vérification locale).
    // Transform/clip/enfants non court-circuités : le
    // sous-arbre visible s'il existe doit garder sa matrice
    // et son clip.
    let hidden = matches!(bx.style.get("visibility"), Some("hidden" | "collapse"));
    if hidden {
        let p = cx.path.clone();
        cx.stat(|s| {
            s.skip_visibility += 1;
            s.visibility_path.get_or_insert(p);
        });
    }
    if !hidden {
        paint_self(bx, items, off, cx.vp, cx.images, cx.canvas_bg_src);
        paint_replaced(bx, items, off, cx.images);
    }
    // `ListItem` ouvre aussi : sa puce est du contenu même
    // sans ligne ni enfant (`<li></li>` peint son disc).
    if !bx.children.is_empty() || !bx.lines.is_empty() || bx.kind == BoxKind::ListItem {
        // Clip sur le padding box (approx : border box moins les
        // bordures — on garde le rect entier, documenté).
        // `bx.lines` compte pour l'ouverture : les lignes
        // propres sont du contenu autant que les enfants (R-55).
        // R-120 : clip si un des DEUX axes est non-visible
        // (`overflow-x:hidden` seul clippe la padding box —
        // l'axe y est devenu `auto`, scrollable).
        let clipped = bx.overflow != Overflow::Visible || bx.overflow_x != Overflow::Visible;
        if clipped {
            // R-M9 — `border-radius` > 0 → clip arrondi,
            // géométrie arrondie au raster (plus d'excès
            // angulaire de contenu hors des coins).
            let clip_rect = shifted(bx.rect, off);
            // R-212 — le clip entre dans la pile : les porteurs
            // reportés (positionnés, inlines) le re-captureront
            // à l'enfilement pour être rognés au drain.
            let radii = corner_radii(&bx.style, &clip_rect);
            cx.clip_stack.push((clip_rect, radii));
            if radii.iter().any(|r| *r > 0.0) {
                items.push(DisplayItem::PushClipRounded {
                    rect: clip_rect,
                    radii,
                });
            } else {
                items.push(DisplayItem::PushClip(clip_rect));
            }
        }
        // R-212 — borne des clips applicables aux `absolute`
        // descendants : leur bloc conteneur est cette boîte
        // (positionnée) ; les clips empilés jusqu'à elle
        // (le sien compris) les rognent, pas ceux entre eux.
        let saved_pos_floor = cx.pos_floor;
        if bx.position != Position::Static {
            cx.pos_floor = cx.clip_stack.len();
        }
        // Les `fixed` descendants remontent à la file RACINE
        // (`fixed_defer`, drainée hors de tout clip) : leur bloc
        // conteneur est le viewport, un `overflow` ancêtre ne
        // doit jamais les rogner (vol 26 : bandeau cookie
        // indeed rogné par le clip d'un ancêtre). Limite : un
        // ancêtre `transform` devrait en faire le bloc
        // conteneur — non modélisé côté layout.
        // Fragments de lignes propres (racine d'un contexte
        // inline — élément inline positionné, anon…) : du
        // contenu → DANS le clip, pas avec les décorations de
        // `paint_self` émises avant (R-55).
        if !hidden {
            emit_list_marker(bx, items, off, cx.images);
            // R-181 — `column-rule` : traits décoratifs entre
            // colonnes au-dessus du fond (§5).
            emit_column_rules(bx, items, off);
            paint_lines(bx, items, off);
        }
        // Ordre App. E au sein des non positionnés : (a)
        // blocs en flux, (b) flottants, (c) contenu en ligne
        // reporté — les porteurs inline remontent au contexte
        // de pilement englobant (`defer` hérité) sauf si cette
        // boîte en ferme la portée (`inline_scope`). Les
        // positionnés à `z-index` **négatif** passent avant
        // les blocs en flux (étape 2 de l'ordre de peinture —
        // le fond d'erreur `z-index:-1` des reftests WPT se
        // peint sous la grille, pas dessus).
        let mut scoped_defer: Vec<(Vec<u16>, &LayoutBox)> = Vec::new();
        let d: &mut Vec<(Vec<u16>, &LayoutBox)> = if inline_scope(bx) {
            &mut scoped_defer
        } else {
            defer
        };
        // File des positionnés `z ≥ 0` : le contexte d'empilement
        // englobant les peint après TOUT le flux normal — un
        // parent `position:relative` à `z-index:auto` ne ferme
        // pas leur phase (App. E). `my_pos` seulement si cette
        // boîte crée le contexte ; sinon file héritée.
        let mut my_pos: Vec<(Vec<u16>, &LayoutBox)> = Vec::new();
        let pd: &mut Vec<(Vec<u16>, &LayoutBox)> = if stacking_ctx(bx) {
            &mut my_pos
        } else {
            pos_defer
        };
        let bound = bx.rect;
        // R-134 : `sticky` est un positionné (css-position —
        // peint comme `relative`, App. E étape 8), pas un
        // élément en flux. Le laisser dans `in_flow` faisait
        // peindre un en-tête sticky AVANT la couche absolue
        // d'une section voisine → en-tête recouvert (smear
        // jaune wttj §B-36 : `fdTXRG_layout-header` sticky +
        // `z-998` ignoré, fond hero absolu par-dessus).
        let in_flow = |c: &&LayoutBox| matches!(c.position, Position::Static);
        // Les deltas sticky se règlent ICI : le `bound` (bloc
        // conteneur = cette boîte) n'est connu que du parent —
        // la file reportée les consomme au drain, quel que soit
        // le contexte d'empilement englobant atteint.
        for child in &bx.children {
            if child.position == Position::Sticky {
                let d = sticky_offset(child, off, bound, cx.vp);
                if d != (0.0, 0.0) {
                    cx.sticky_deltas
                        .insert(std::ptr::from_ref(child) as usize, d);
                }
            }
            // R-272 — `offset-path` : le chemin se trace dans
            // le bloc conteneur (padding box du parent, §5.2)
            // — calculé ici, consommé au `box_transform` de
            // l'enfant quel que soit le drain (reporté ou non).
            if child.style.get("offset-path").is_some_and(|v| {
                let v = v.trim();
                !v.is_empty() && v != "none"
            }) {
                let cb = shifted(content_rect(bx), off);
                let rect = shifted(child.rect, off);
                if let Some(t) =
                    explorer_layout::transform::offset_transform(&child.style, &rect, &cb)
                {
                    cx.offset_tf.insert(std::ptr::from_ref(child) as usize, t);
                }
            }
        }
        let mut neg_z: Vec<(usize, &LayoutBox)> = bx
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| !in_flow(c) && c.z_index < 0)
            .collect();
        neg_z.sort_by_key(|(_, c)| c.z_index);
        for (ci, child) in neg_z {
            if child.position == Position::Fixed {
                // `fixed` → file racine : échappe au clip
                // `overflow` ancêtre (voir `fixed_defer`).
                let mut p = cx.path.clone();
                p.push(u16::try_from(ci).unwrap_or(u16::MAX));
                push_z(fixed_defer, 0, p, child);
            } else {
                emit_scoped(ci, child, items, off, cx, pd, fixed_defer);
            }
        }
        // R-183 — `writing-mode` orthogonal : les enfants
        // in-flow ont été posés en coords transposées — le
        // `PushTransform` du repère de flux les remet dans le
        // repère parent (miroir de `hit_test_m`/`paint_bounds`).
        // Enfants positionnés/absolus : hors portée —
        // limitation documentée (comme les `pos_defer` qui
        // échappent aux transforms CSS).
        let flow_t = {
            let own =
                explorer_css::value::parse_writing_mode(bx.style.get("writing-mode").unwrap_or(""));
            explorer_layout::transform::flow_transform(bx.flow_mode, own, &content_rect(bx))
                .map(|t| tiny_skia::Transform::from_row(t.a, t.b, t.c, t.d, t.e, t.f))
        };
        if let Some(t) = flow_t {
            items.push(DisplayItem::PushTransform(t));
        }
        // (a) blocs en flux ; récursion `deep` ∝ profondeur
        // (R-24). Les sticky sont positionnés depuis R-134 —
        // leur translation part dans la file (`sticky_deltas`).
        for (ci, child) in bx.children.iter().enumerate() {
            if !in_flow(&child) || child.float != Float::None || inline_carrier(child) {
                continue;
            }
            emit_child(ci, child, items, off, cx, d, pd, fixed_defer);
        }
        // (b) flottants non positionnés — portée propre.
        for (ci, child) in bx.children.iter().enumerate() {
            if in_flow(&child) && child.float != Float::None {
                emit_scoped(ci, child, items, off, cx, pd, fixed_defer);
            }
        }
        // (c) porteurs inline — reportés au contexte englobant.
        for (ci, child) in bx.children.iter().enumerate() {
            if in_flow(&child) && inline_carrier(child) {
                // Le chemin complet est capturé ici — le drain
                // peut avoir lieu chez un ancêtre englobant.
                let mut p = cx.path.clone();
                p.push(u16::try_from(ci).unwrap_or(u16::MAX));
                // R-212 — porteur en flux : tous les clips
                // ancêtres s'appliquent (§11.1.1).
                cx.defer_clips
                    .insert(p.clone().into_boxed_slice(), cx.clip_stack.clone().into());
                d.push((p, child));
            }
        }
        // Si cette boîte ferme la portée : drainer ses inlines
        // dans l'ordre de collecte (étape 5).
        let mut i = 0;
        while i < scoped_defer.len() {
            let (p, c) = scoped_defer[i].clone();
            i += 1;
            emit_scoped_at(p, c, items, off, cx, pd, fixed_defer);
        }
        if flow_t.is_some() {
            items.push(DisplayItem::PopTransform);
        }
        // Enfants positionnés `z-index` ≥ 0 : enfilés triés dans
        // la file du contexte — drainés à sa fermeture (ici si
        // `my_pos`), sinon au contexte ancêtre ou à la racine.
        for (ci, child) in bx.children.iter().enumerate() {
            if in_flow(&child) || child.z_index < 0 {
                continue;
            }
            let mut p = cx.path.clone();
            p.push(u16::try_from(ci).unwrap_or(u16::MAX));
            if child.position == Position::Fixed {
                push_z(fixed_defer, 0, p, child);
            } else {
                // R-212 — clips applicables : tout pour un
                // in-flow positionné ; pour un `absolute`,
                // borné à son bloc conteneur (§11.1.1).
                let clips = if child.position == Position::Absolute {
                    cx.clip_stack[..cx.pos_floor].to_vec().into()
                } else {
                    cx.clip_stack.clone().into()
                };
                cx.defer_clips.insert(p.clone().into_boxed_slice(), clips);
                push_z(pd, 0, p, child);
            }
        }
        // Même discipline que le drain racine : les poussées
        // pendant l'émission passent par `pending` et rejoignent
        // la file triée au-delà du curseur (§B-26).
        let mut pending = Vec::new();
        let mut i = 0;
        while i < my_pos.len() {
            let (p, c) = my_pos[i].clone();
            i += 1;
            emit_scoped_at(p, c, items, off, cx, &mut pending, fixed_defer);
            for (p, c) in std::mem::take(&mut pending) {
                push_z(&mut my_pos, i, p, c);
            }
        }
        if clipped {
            items.push(DisplayItem::PopClip);
            cx.clip_stack.pop();
        }
        cx.pos_floor = saved_pos_floor;
    }
    if geo_clip.is_some() {
        items.push(DisplayItem::PopClip);
    }
    if tf.is_some() {
        items.push(DisplayItem::PopTransform);
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_rects, rect_union};
    use explorer_layout::Rect;
    use std::time::Instant;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    /// Aucun rect ne chevauche `a` directement, mais `b ∪ c`
    /// le couvre — l'ancienne passe unique laissait `a`
    /// séparé (jamais de point fixe).
    #[test]
    fn merge_point_fixe_pont_bbox() {
        let mut v = vec![
            r(0.0, 0.0, 10.0, 10.0),
            r(0.0, 15.0, 30.0, 5.0),
            r(20.0, 0.0, 10.0, 16.0),
        ];
        merge_rects(&mut v);
        assert_eq!(v.len(), 1);
        let u = v[0];
        assert_eq!((u.x, u.y, u.width, u.height), (0.0, 0.0, 30.0, 20.0));
    }

    #[test]
    fn merge_preserve_la_couverture() {
        let src = [
            r(0.0, 0.0, 5.0, 5.0),
            r(4.0, 4.0, 5.0, 5.0),
            r(40.0, 0.0, 8.0, 8.0),
            r(40.0, 8.0, 8.0, 8.0),
            r(0.0, 40.0, 4.0, 4.0),
        ];
        let total = src.iter().copied().reduce(rect_union);
        let mut v = src.to_vec();
        merge_rects(&mut v);
        let merged = v.iter().copied().reduce(rect_union);
        assert_eq!(merged, total);
        // Disjoints non chaînés : jamais de fusion abusive.
        assert!(v.len() <= src.len());
    }

    /// R-76 — les composantes fermées par la compaction
    /// rejoignent la sortie : avant, `retain` les jetait →
    /// leurs zones n'entraient jamais dans le damage
    /// (pixels changés jamais repeints — issue #11 bis,
    /// lobste.rs : in=2060 out=39 au lieu de ~505).
    #[test]
    fn merge_compaction_reemet_les_fermees() {
        // 70 rects disjoints à gauche → 70 composantes
        // ouvertes ; un rect loin à droite les ferme toutes
        // d'un coup → compaction (mortes majoritaires, >64).
        let mut v: Vec<Rect> = (0..70_u16)
            .map(|i| r(0.0, f32::from(i) * 20.0, 10.0, 10.0))
            .collect();
        v.push(r(500.0, 0.0, 10.0, 10.0));
        let area: f32 = v.iter().map(|q| q.width * q.height).sum();
        merge_rects(&mut v);
        // Avant le fix : un seul rect survivait.
        assert_eq!(v.len(), 71, "composantes fermées jetées");
        let covered: f32 = v.iter().map(|q| q.width * q.height).sum();
        assert_eq!(covered, area, "couverture perdue");
    }

    /// Le diff dégénéré journalisé par NAV : ~5 000 rects en
    /// chaîne diagonale — l'ancien O(k²) coûtait ~2 s sur
    /// ~8 500 ; le balayage doit rester très en deçà.
    #[test]
    fn merge_degenere_reste_rapide() {
        let mut v: Vec<Rect> = (0..5_000_u16)
            .map(|i| r(f32::from(i) * 8.0, (f32::from(i) * 8.0) % 600.0, 20.0, 20.0))
            .collect();
        let t0 = Instant::now();
        merge_rects(&mut v);
        assert!(
            t0.elapsed().as_secs() < 2,
            "merge_rects trop lent : {:?}",
            t0.elapsed()
        );
    }

    /// Erratum R-286 (régression re-tour NAV) : avec une
    /// fonte dont l'encre déclarée couvre toute la bande du
    /// trait — Ahem, fontes plein-em — le skip-ink ne doit
    /// **pas** annihiler le `DecoRect` entier : `auto` est
    /// un « should », encre pathologique → trait plein.
    #[test]
    fn skip_ink_encre_pathologique_garde_le_trait() {
        use super::deco_keep_ranges;
        use explorer_layout::font::{FontMetrics, Glyph, ShapedRun};
        let mk = || Glyph {
            id: 1,
            advance: 10.0,
            offset_x: 0.0,
            offset_y: 0.0,
            cluster: 0,
            invisible: false,
        };
        // Encre plein-em : chaque glyphe couvre la bande.
        let run = ShapedRun {
            glyphs: vec![mk(), mk()],
            advance: 20.0,
            metrics: FontMetrics {
                ascent: 8.0,
                descent: -2.0,
                line_gap: 0.0,
                units_per_em: 1000,
            },
            fallback_spans: Vec::new(),
            ink: None,
            ink_glyphs: vec![r(0.0, -8.0, 10.0, 10.0), r(10.0, -8.0, 10.0, 10.0)],
        };
        // Bande [1,2] sous la baseline — traversée par les
        // deux encre — sans garde-fou : `keep` vide.
        let keep = deco_keep_ranges("ab", &run, 20.0, 0, Some((1.0, 2.0)));
        assert_eq!(keep, vec![(0.0, 20.0)], "encre plein-em → trait plein");
        // Encre partielle (un glyphe n'atteint pas la
        // bande) : le skip-ink normal continue de trouer.
        let mut run2 = run.clone();
        run2.ink_glyphs[1] = r(10.0, -8.0, 10.0, 5.0);
        let keep2 = deco_keep_ranges("ab", &run2, 20.0, 0, Some((1.0, 2.0)));
        assert_eq!(keep2, vec![(10.0, 20.0)], "encre saine → trou conservé");
    }
}
