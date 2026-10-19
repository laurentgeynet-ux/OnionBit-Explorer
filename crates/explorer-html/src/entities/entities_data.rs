// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Table GENEREE depuis `entities.json` WHATWG (html.spec.whatwg.org) —
//! les ~2 200 references de caracteres nommees de la spec (R-15).
//! Ne pas editer a la main : regenerer depuis la source officielle.

use super::NamedRef;

/// Entites nommees WHATWG — triees par nom.
pub(super) static NAMED_REFS: &[NamedRef] = &[
    NamedRef {
        name: "AElig",
        value: "\u{C6}",
        legacy: true,
    },
    NamedRef {
        name: "AMP",
        value: "\u{26}",
        legacy: true,
    },
    NamedRef {
        name: "Aacute",
        value: "\u{C1}",
        legacy: true,
    },
    NamedRef {
        name: "Abreve",
        value: "\u{102}",
        legacy: false,
    },
    NamedRef {
        name: "Acirc",
        value: "\u{C2}",
        legacy: true,
    },
    NamedRef {
        name: "Acy",
        value: "\u{410}",
        legacy: false,
    },
    NamedRef {
        name: "Afr",
        value: "\u{1D504}",
        legacy: false,
    },
    NamedRef {
        name: "Agrave",
        value: "\u{C0}",
        legacy: true,
    },
    NamedRef {
        name: "Alpha",
        value: "\u{391}",
        legacy: false,
    },
    NamedRef {
        name: "Amacr",
        value: "\u{100}",
        legacy: false,
    },
    NamedRef {
        name: "And",
        value: "\u{2A53}",
        legacy: false,
    },
    NamedRef {
        name: "Aogon",
        value: "\u{104}",
        legacy: false,
    },
    NamedRef {
        name: "Aopf",
        value: "\u{1D538}",
        legacy: false,
    },
    NamedRef {
        name: "ApplyFunction",
        value: "\u{2061}",
        legacy: false,
    },
    NamedRef {
        name: "Aring",
        value: "\u{C5}",
        legacy: true,
    },
    NamedRef {
        name: "Ascr",
        value: "\u{1D49C}",
        legacy: false,
    },
    NamedRef {
        name: "Assign",
        value: "\u{2254}",
        legacy: false,
    },
    NamedRef {
        name: "Atilde",
        value: "\u{C3}",
        legacy: true,
    },
    NamedRef {
        name: "Auml",
        value: "\u{C4}",
        legacy: true,
    },
    NamedRef {
        name: "Backslash",
        value: "\u{2216}",
        legacy: false,
    },
    NamedRef {
        name: "Barv",
        value: "\u{2AE7}",
        legacy: false,
    },
    NamedRef {
        name: "Barwed",
        value: "\u{2306}",
        legacy: false,
    },
    NamedRef {
        name: "Bcy",
        value: "\u{411}",
        legacy: false,
    },
    NamedRef {
        name: "Because",
        value: "\u{2235}",
        legacy: false,
    },
    NamedRef {
        name: "Bernoullis",
        value: "\u{212C}",
        legacy: false,
    },
    NamedRef {
        name: "Beta",
        value: "\u{392}",
        legacy: false,
    },
    NamedRef {
        name: "Bfr",
        value: "\u{1D505}",
        legacy: false,
    },
    NamedRef {
        name: "Bopf",
        value: "\u{1D539}",
        legacy: false,
    },
    NamedRef {
        name: "Breve",
        value: "\u{2D8}",
        legacy: false,
    },
    NamedRef {
        name: "Bscr",
        value: "\u{212C}",
        legacy: false,
    },
    NamedRef {
        name: "Bumpeq",
        value: "\u{224E}",
        legacy: false,
    },
    NamedRef {
        name: "CHcy",
        value: "\u{427}",
        legacy: false,
    },
    NamedRef {
        name: "COPY",
        value: "\u{A9}",
        legacy: true,
    },
    NamedRef {
        name: "Cacute",
        value: "\u{106}",
        legacy: false,
    },
    NamedRef {
        name: "Cap",
        value: "\u{22D2}",
        legacy: false,
    },
    NamedRef {
        name: "CapitalDifferentialD",
        value: "\u{2145}",
        legacy: false,
    },
    NamedRef {
        name: "Cayleys",
        value: "\u{212D}",
        legacy: false,
    },
    NamedRef {
        name: "Ccaron",
        value: "\u{10C}",
        legacy: false,
    },
    NamedRef {
        name: "Ccedil",
        value: "\u{C7}",
        legacy: true,
    },
    NamedRef {
        name: "Ccirc",
        value: "\u{108}",
        legacy: false,
    },
    NamedRef {
        name: "Cconint",
        value: "\u{2230}",
        legacy: false,
    },
    NamedRef {
        name: "Cdot",
        value: "\u{10A}",
        legacy: false,
    },
    NamedRef {
        name: "Cedilla",
        value: "\u{B8}",
        legacy: false,
    },
    NamedRef {
        name: "CenterDot",
        value: "\u{B7}",
        legacy: false,
    },
    NamedRef {
        name: "Cfr",
        value: "\u{212D}",
        legacy: false,
    },
    NamedRef {
        name: "Chi",
        value: "\u{3A7}",
        legacy: false,
    },
    NamedRef {
        name: "CircleDot",
        value: "\u{2299}",
        legacy: false,
    },
    NamedRef {
        name: "CircleMinus",
        value: "\u{2296}",
        legacy: false,
    },
    NamedRef {
        name: "CirclePlus",
        value: "\u{2295}",
        legacy: false,
    },
    NamedRef {
        name: "CircleTimes",
        value: "\u{2297}",
        legacy: false,
    },
    NamedRef {
        name: "ClockwiseContourIntegral",
        value: "\u{2232}",
        legacy: false,
    },
    NamedRef {
        name: "CloseCurlyDoubleQuote",
        value: "\u{201D}",
        legacy: false,
    },
    NamedRef {
        name: "CloseCurlyQuote",
        value: "\u{2019}",
        legacy: false,
    },
    NamedRef {
        name: "Colon",
        value: "\u{2237}",
        legacy: false,
    },
    NamedRef {
        name: "Colone",
        value: "\u{2A74}",
        legacy: false,
    },
    NamedRef {
        name: "Congruent",
        value: "\u{2261}",
        legacy: false,
    },
    NamedRef {
        name: "Conint",
        value: "\u{222F}",
        legacy: false,
    },
    NamedRef {
        name: "ContourIntegral",
        value: "\u{222E}",
        legacy: false,
    },
    NamedRef {
        name: "Copf",
        value: "\u{2102}",
        legacy: false,
    },
    NamedRef {
        name: "Coproduct",
        value: "\u{2210}",
        legacy: false,
    },
    NamedRef {
        name: "CounterClockwiseContourIntegral",
        value: "\u{2233}",
        legacy: false,
    },
    NamedRef {
        name: "Cross",
        value: "\u{2A2F}",
        legacy: false,
    },
    NamedRef {
        name: "Cscr",
        value: "\u{1D49E}",
        legacy: false,
    },
    NamedRef {
        name: "Cup",
        value: "\u{22D3}",
        legacy: false,
    },
    NamedRef {
        name: "CupCap",
        value: "\u{224D}",
        legacy: false,
    },
    NamedRef {
        name: "DD",
        value: "\u{2145}",
        legacy: false,
    },
    NamedRef {
        name: "DDotrahd",
        value: "\u{2911}",
        legacy: false,
    },
    NamedRef {
        name: "DJcy",
        value: "\u{402}",
        legacy: false,
    },
    NamedRef {
        name: "DScy",
        value: "\u{405}",
        legacy: false,
    },
    NamedRef {
        name: "DZcy",
        value: "\u{40F}",
        legacy: false,
    },
    NamedRef {
        name: "Dagger",
        value: "\u{2021}",
        legacy: false,
    },
    NamedRef {
        name: "Darr",
        value: "\u{21A1}",
        legacy: false,
    },
    NamedRef {
        name: "Dashv",
        value: "\u{2AE4}",
        legacy: false,
    },
    NamedRef {
        name: "Dcaron",
        value: "\u{10E}",
        legacy: false,
    },
    NamedRef {
        name: "Dcy",
        value: "\u{414}",
        legacy: false,
    },
    NamedRef {
        name: "Del",
        value: "\u{2207}",
        legacy: false,
    },
    NamedRef {
        name: "Delta",
        value: "\u{394}",
        legacy: false,
    },
    NamedRef {
        name: "Dfr",
        value: "\u{1D507}",
        legacy: false,
    },
    NamedRef {
        name: "DiacriticalAcute",
        value: "\u{B4}",
        legacy: false,
    },
    NamedRef {
        name: "DiacriticalDot",
        value: "\u{2D9}",
        legacy: false,
    },
    NamedRef {
        name: "DiacriticalDoubleAcute",
        value: "\u{2DD}",
        legacy: false,
    },
    NamedRef {
        name: "DiacriticalGrave",
        value: "\u{60}",
        legacy: false,
    },
    NamedRef {
        name: "DiacriticalTilde",
        value: "\u{2DC}",
        legacy: false,
    },
    NamedRef {
        name: "Diamond",
        value: "\u{22C4}",
        legacy: false,
    },
    NamedRef {
        name: "DifferentialD",
        value: "\u{2146}",
        legacy: false,
    },
    NamedRef {
        name: "Dopf",
        value: "\u{1D53B}",
        legacy: false,
    },
    NamedRef {
        name: "Dot",
        value: "\u{A8}",
        legacy: false,
    },
    NamedRef {
        name: "DotDot",
        value: "\u{20DC}",
        legacy: false,
    },
    NamedRef {
        name: "DotEqual",
        value: "\u{2250}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleContourIntegral",
        value: "\u{222F}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleDot",
        value: "\u{A8}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleDownArrow",
        value: "\u{21D3}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleLeftArrow",
        value: "\u{21D0}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleLeftRightArrow",
        value: "\u{21D4}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleLeftTee",
        value: "\u{2AE4}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleLongLeftArrow",
        value: "\u{27F8}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleLongLeftRightArrow",
        value: "\u{27FA}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleLongRightArrow",
        value: "\u{27F9}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleRightArrow",
        value: "\u{21D2}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleRightTee",
        value: "\u{22A8}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleUpArrow",
        value: "\u{21D1}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleUpDownArrow",
        value: "\u{21D5}",
        legacy: false,
    },
    NamedRef {
        name: "DoubleVerticalBar",
        value: "\u{2225}",
        legacy: false,
    },
    NamedRef {
        name: "DownArrow",
        value: "\u{2193}",
        legacy: false,
    },
    NamedRef {
        name: "DownArrowBar",
        value: "\u{2913}",
        legacy: false,
    },
    NamedRef {
        name: "DownArrowUpArrow",
        value: "\u{21F5}",
        legacy: false,
    },
    NamedRef {
        name: "DownBreve",
        value: "\u{311}",
        legacy: false,
    },
    NamedRef {
        name: "DownLeftRightVector",
        value: "\u{2950}",
        legacy: false,
    },
    NamedRef {
        name: "DownLeftTeeVector",
        value: "\u{295E}",
        legacy: false,
    },
    NamedRef {
        name: "DownLeftVector",
        value: "\u{21BD}",
        legacy: false,
    },
    NamedRef {
        name: "DownLeftVectorBar",
        value: "\u{2956}",
        legacy: false,
    },
    NamedRef {
        name: "DownRightTeeVector",
        value: "\u{295F}",
        legacy: false,
    },
    NamedRef {
        name: "DownRightVector",
        value: "\u{21C1}",
        legacy: false,
    },
    NamedRef {
        name: "DownRightVectorBar",
        value: "\u{2957}",
        legacy: false,
    },
    NamedRef {
        name: "DownTee",
        value: "\u{22A4}",
        legacy: false,
    },
    NamedRef {
        name: "DownTeeArrow",
        value: "\u{21A7}",
        legacy: false,
    },
    NamedRef {
        name: "Downarrow",
        value: "\u{21D3}",
        legacy: false,
    },
    NamedRef {
        name: "Dscr",
        value: "\u{1D49F}",
        legacy: false,
    },
    NamedRef {
        name: "Dstrok",
        value: "\u{110}",
        legacy: false,
    },
    NamedRef {
        name: "ENG",
        value: "\u{14A}",
        legacy: false,
    },
    NamedRef {
        name: "ETH",
        value: "\u{D0}",
        legacy: true,
    },
    NamedRef {
        name: "Eacute",
        value: "\u{C9}",
        legacy: true,
    },
    NamedRef {
        name: "Ecaron",
        value: "\u{11A}",
        legacy: false,
    },
    NamedRef {
        name: "Ecirc",
        value: "\u{CA}",
        legacy: true,
    },
    NamedRef {
        name: "Ecy",
        value: "\u{42D}",
        legacy: false,
    },
    NamedRef {
        name: "Edot",
        value: "\u{116}",
        legacy: false,
    },
    NamedRef {
        name: "Efr",
        value: "\u{1D508}",
        legacy: false,
    },
    NamedRef {
        name: "Egrave",
        value: "\u{C8}",
        legacy: true,
    },
    NamedRef {
        name: "Element",
        value: "\u{2208}",
        legacy: false,
    },
    NamedRef {
        name: "Emacr",
        value: "\u{112}",
        legacy: false,
    },
    NamedRef {
        name: "EmptySmallSquare",
        value: "\u{25FB}",
        legacy: false,
    },
    NamedRef {
        name: "EmptyVerySmallSquare",
        value: "\u{25AB}",
        legacy: false,
    },
    NamedRef {
        name: "Eogon",
        value: "\u{118}",
        legacy: false,
    },
    NamedRef {
        name: "Eopf",
        value: "\u{1D53C}",
        legacy: false,
    },
    NamedRef {
        name: "Epsilon",
        value: "\u{395}",
        legacy: false,
    },
    NamedRef {
        name: "Equal",
        value: "\u{2A75}",
        legacy: false,
    },
    NamedRef {
        name: "EqualTilde",
        value: "\u{2242}",
        legacy: false,
    },
    NamedRef {
        name: "Equilibrium",
        value: "\u{21CC}",
        legacy: false,
    },
    NamedRef {
        name: "Escr",
        value: "\u{2130}",
        legacy: false,
    },
    NamedRef {
        name: "Esim",
        value: "\u{2A73}",
        legacy: false,
    },
    NamedRef {
        name: "Eta",
        value: "\u{397}",
        legacy: false,
    },
    NamedRef {
        name: "Euml",
        value: "\u{CB}",
        legacy: true,
    },
    NamedRef {
        name: "Exists",
        value: "\u{2203}",
        legacy: false,
    },
    NamedRef {
        name: "ExponentialE",
        value: "\u{2147}",
        legacy: false,
    },
    NamedRef {
        name: "Fcy",
        value: "\u{424}",
        legacy: false,
    },
    NamedRef {
        name: "Ffr",
        value: "\u{1D509}",
        legacy: false,
    },
    NamedRef {
        name: "FilledSmallSquare",
        value: "\u{25FC}",
        legacy: false,
    },
    NamedRef {
        name: "FilledVerySmallSquare",
        value: "\u{25AA}",
        legacy: false,
    },
    NamedRef {
        name: "Fopf",
        value: "\u{1D53D}",
        legacy: false,
    },
    NamedRef {
        name: "ForAll",
        value: "\u{2200}",
        legacy: false,
    },
    NamedRef {
        name: "Fouriertrf",
        value: "\u{2131}",
        legacy: false,
    },
    NamedRef {
        name: "Fscr",
        value: "\u{2131}",
        legacy: false,
    },
    NamedRef {
        name: "GJcy",
        value: "\u{403}",
        legacy: false,
    },
    NamedRef {
        name: "GT",
        value: "\u{3E}",
        legacy: true,
    },
    NamedRef {
        name: "Gamma",
        value: "\u{393}",
        legacy: false,
    },
    NamedRef {
        name: "Gammad",
        value: "\u{3DC}",
        legacy: false,
    },
    NamedRef {
        name: "Gbreve",
        value: "\u{11E}",
        legacy: false,
    },
    NamedRef {
        name: "Gcedil",
        value: "\u{122}",
        legacy: false,
    },
    NamedRef {
        name: "Gcirc",
        value: "\u{11C}",
        legacy: false,
    },
    NamedRef {
        name: "Gcy",
        value: "\u{413}",
        legacy: false,
    },
    NamedRef {
        name: "Gdot",
        value: "\u{120}",
        legacy: false,
    },
    NamedRef {
        name: "Gfr",
        value: "\u{1D50A}",
        legacy: false,
    },
    NamedRef {
        name: "Gg",
        value: "\u{22D9}",
        legacy: false,
    },
    NamedRef {
        name: "Gopf",
        value: "\u{1D53E}",
        legacy: false,
    },
    NamedRef {
        name: "GreaterEqual",
        value: "\u{2265}",
        legacy: false,
    },
    NamedRef {
        name: "GreaterEqualLess",
        value: "\u{22DB}",
        legacy: false,
    },
    NamedRef {
        name: "GreaterFullEqual",
        value: "\u{2267}",
        legacy: false,
    },
    NamedRef {
        name: "GreaterGreater",
        value: "\u{2AA2}",
        legacy: false,
    },
    NamedRef {
        name: "GreaterLess",
        value: "\u{2277}",
        legacy: false,
    },
    NamedRef {
        name: "GreaterSlantEqual",
        value: "\u{2A7E}",
        legacy: false,
    },
    NamedRef {
        name: "GreaterTilde",
        value: "\u{2273}",
        legacy: false,
    },
    NamedRef {
        name: "Gscr",
        value: "\u{1D4A2}",
        legacy: false,
    },
    NamedRef {
        name: "Gt",
        value: "\u{226B}",
        legacy: false,
    },
    NamedRef {
        name: "HARDcy",
        value: "\u{42A}",
        legacy: false,
    },
    NamedRef {
        name: "Hacek",
        value: "\u{2C7}",
        legacy: false,
    },
    NamedRef {
        name: "Hat",
        value: "\u{5E}",
        legacy: false,
    },
    NamedRef {
        name: "Hcirc",
        value: "\u{124}",
        legacy: false,
    },
    NamedRef {
        name: "Hfr",
        value: "\u{210C}",
        legacy: false,
    },
    NamedRef {
        name: "HilbertSpace",
        value: "\u{210B}",
        legacy: false,
    },
    NamedRef {
        name: "Hopf",
        value: "\u{210D}",
        legacy: false,
    },
    NamedRef {
        name: "HorizontalLine",
        value: "\u{2500}",
        legacy: false,
    },
    NamedRef {
        name: "Hscr",
        value: "\u{210B}",
        legacy: false,
    },
    NamedRef {
        name: "Hstrok",
        value: "\u{126}",
        legacy: false,
    },
    NamedRef {
        name: "HumpDownHump",
        value: "\u{224E}",
        legacy: false,
    },
    NamedRef {
        name: "HumpEqual",
        value: "\u{224F}",
        legacy: false,
    },
    NamedRef {
        name: "IEcy",
        value: "\u{415}",
        legacy: false,
    },
    NamedRef {
        name: "IJlig",
        value: "\u{132}",
        legacy: false,
    },
    NamedRef {
        name: "IOcy",
        value: "\u{401}",
        legacy: false,
    },
    NamedRef {
        name: "Iacute",
        value: "\u{CD}",
        legacy: true,
    },
    NamedRef {
        name: "Icirc",
        value: "\u{CE}",
        legacy: true,
    },
    NamedRef {
        name: "Icy",
        value: "\u{418}",
        legacy: false,
    },
    NamedRef {
        name: "Idot",
        value: "\u{130}",
        legacy: false,
    },
    NamedRef {
        name: "Ifr",
        value: "\u{2111}",
        legacy: false,
    },
    NamedRef {
        name: "Igrave",
        value: "\u{CC}",
        legacy: true,
    },
    NamedRef {
        name: "Im",
        value: "\u{2111}",
        legacy: false,
    },
    NamedRef {
        name: "Imacr",
        value: "\u{12A}",
        legacy: false,
    },
    NamedRef {
        name: "ImaginaryI",
        value: "\u{2148}",
        legacy: false,
    },
    NamedRef {
        name: "Implies",
        value: "\u{21D2}",
        legacy: false,
    },
    NamedRef {
        name: "Int",
        value: "\u{222C}",
        legacy: false,
    },
    NamedRef {
        name: "Integral",
        value: "\u{222B}",
        legacy: false,
    },
    NamedRef {
        name: "Intersection",
        value: "\u{22C2}",
        legacy: false,
    },
    NamedRef {
        name: "InvisibleComma",
        value: "\u{2063}",
        legacy: false,
    },
    NamedRef {
        name: "InvisibleTimes",
        value: "\u{2062}",
        legacy: false,
    },
    NamedRef {
        name: "Iogon",
        value: "\u{12E}",
        legacy: false,
    },
    NamedRef {
        name: "Iopf",
        value: "\u{1D540}",
        legacy: false,
    },
    NamedRef {
        name: "Iota",
        value: "\u{399}",
        legacy: false,
    },
    NamedRef {
        name: "Iscr",
        value: "\u{2110}",
        legacy: false,
    },
    NamedRef {
        name: "Itilde",
        value: "\u{128}",
        legacy: false,
    },
    NamedRef {
        name: "Iukcy",
        value: "\u{406}",
        legacy: false,
    },
    NamedRef {
        name: "Iuml",
        value: "\u{CF}",
        legacy: true,
    },
    NamedRef {
        name: "Jcirc",
        value: "\u{134}",
        legacy: false,
    },
    NamedRef {
        name: "Jcy",
        value: "\u{419}",
        legacy: false,
    },
    NamedRef {
        name: "Jfr",
        value: "\u{1D50D}",
        legacy: false,
    },
    NamedRef {
        name: "Jopf",
        value: "\u{1D541}",
        legacy: false,
    },
    NamedRef {
        name: "Jscr",
        value: "\u{1D4A5}",
        legacy: false,
    },
    NamedRef {
        name: "Jsercy",
        value: "\u{408}",
        legacy: false,
    },
    NamedRef {
        name: "Jukcy",
        value: "\u{404}",
        legacy: false,
    },
    NamedRef {
        name: "KHcy",
        value: "\u{425}",
        legacy: false,
    },
    NamedRef {
        name: "KJcy",
        value: "\u{40C}",
        legacy: false,
    },
    NamedRef {
        name: "Kappa",
        value: "\u{39A}",
        legacy: false,
    },
    NamedRef {
        name: "Kcedil",
        value: "\u{136}",
        legacy: false,
    },
    NamedRef {
        name: "Kcy",
        value: "\u{41A}",
        legacy: false,
    },
    NamedRef {
        name: "Kfr",
        value: "\u{1D50E}",
        legacy: false,
    },
    NamedRef {
        name: "Kopf",
        value: "\u{1D542}",
        legacy: false,
    },
    NamedRef {
        name: "Kscr",
        value: "\u{1D4A6}",
        legacy: false,
    },
    NamedRef {
        name: "LJcy",
        value: "\u{409}",
        legacy: false,
    },
    NamedRef {
        name: "LT",
        value: "\u{3C}",
        legacy: true,
    },
    NamedRef {
        name: "Lacute",
        value: "\u{139}",
        legacy: false,
    },
    NamedRef {
        name: "Lambda",
        value: "\u{39B}",
        legacy: false,
    },
    NamedRef {
        name: "Lang",
        value: "\u{27EA}",
        legacy: false,
    },
    NamedRef {
        name: "Laplacetrf",
        value: "\u{2112}",
        legacy: false,
    },
    NamedRef {
        name: "Larr",
        value: "\u{219E}",
        legacy: false,
    },
    NamedRef {
        name: "Lcaron",
        value: "\u{13D}",
        legacy: false,
    },
    NamedRef {
        name: "Lcedil",
        value: "\u{13B}",
        legacy: false,
    },
    NamedRef {
        name: "Lcy",
        value: "\u{41B}",
        legacy: false,
    },
    NamedRef {
        name: "LeftAngleBracket",
        value: "\u{27E8}",
        legacy: false,
    },
    NamedRef {
        name: "LeftArrow",
        value: "\u{2190}",
        legacy: false,
    },
    NamedRef {
        name: "LeftArrowBar",
        value: "\u{21E4}",
        legacy: false,
    },
    NamedRef {
        name: "LeftArrowRightArrow",
        value: "\u{21C6}",
        legacy: false,
    },
    NamedRef {
        name: "LeftCeiling",
        value: "\u{2308}",
        legacy: false,
    },
    NamedRef {
        name: "LeftDoubleBracket",
        value: "\u{27E6}",
        legacy: false,
    },
    NamedRef {
        name: "LeftDownTeeVector",
        value: "\u{2961}",
        legacy: false,
    },
    NamedRef {
        name: "LeftDownVector",
        value: "\u{21C3}",
        legacy: false,
    },
    NamedRef {
        name: "LeftDownVectorBar",
        value: "\u{2959}",
        legacy: false,
    },
    NamedRef {
        name: "LeftFloor",
        value: "\u{230A}",
        legacy: false,
    },
    NamedRef {
        name: "LeftRightArrow",
        value: "\u{2194}",
        legacy: false,
    },
    NamedRef {
        name: "LeftRightVector",
        value: "\u{294E}",
        legacy: false,
    },
    NamedRef {
        name: "LeftTee",
        value: "\u{22A3}",
        legacy: false,
    },
    NamedRef {
        name: "LeftTeeArrow",
        value: "\u{21A4}",
        legacy: false,
    },
    NamedRef {
        name: "LeftTeeVector",
        value: "\u{295A}",
        legacy: false,
    },
    NamedRef {
        name: "LeftTriangle",
        value: "\u{22B2}",
        legacy: false,
    },
    NamedRef {
        name: "LeftTriangleBar",
        value: "\u{29CF}",
        legacy: false,
    },
    NamedRef {
        name: "LeftTriangleEqual",
        value: "\u{22B4}",
        legacy: false,
    },
    NamedRef {
        name: "LeftUpDownVector",
        value: "\u{2951}",
        legacy: false,
    },
    NamedRef {
        name: "LeftUpTeeVector",
        value: "\u{2960}",
        legacy: false,
    },
    NamedRef {
        name: "LeftUpVector",
        value: "\u{21BF}",
        legacy: false,
    },
    NamedRef {
        name: "LeftUpVectorBar",
        value: "\u{2958}",
        legacy: false,
    },
    NamedRef {
        name: "LeftVector",
        value: "\u{21BC}",
        legacy: false,
    },
    NamedRef {
        name: "LeftVectorBar",
        value: "\u{2952}",
        legacy: false,
    },
    NamedRef {
        name: "Leftarrow",
        value: "\u{21D0}",
        legacy: false,
    },
    NamedRef {
        name: "Leftrightarrow",
        value: "\u{21D4}",
        legacy: false,
    },
    NamedRef {
        name: "LessEqualGreater",
        value: "\u{22DA}",
        legacy: false,
    },
    NamedRef {
        name: "LessFullEqual",
        value: "\u{2266}",
        legacy: false,
    },
    NamedRef {
        name: "LessGreater",
        value: "\u{2276}",
        legacy: false,
    },
    NamedRef {
        name: "LessLess",
        value: "\u{2AA1}",
        legacy: false,
    },
    NamedRef {
        name: "LessSlantEqual",
        value: "\u{2A7D}",
        legacy: false,
    },
    NamedRef {
        name: "LessTilde",
        value: "\u{2272}",
        legacy: false,
    },
    NamedRef {
        name: "Lfr",
        value: "\u{1D50F}",
        legacy: false,
    },
    NamedRef {
        name: "Ll",
        value: "\u{22D8}",
        legacy: false,
    },
    NamedRef {
        name: "Lleftarrow",
        value: "\u{21DA}",
        legacy: false,
    },
    NamedRef {
        name: "Lmidot",
        value: "\u{13F}",
        legacy: false,
    },
    NamedRef {
        name: "LongLeftArrow",
        value: "\u{27F5}",
        legacy: false,
    },
    NamedRef {
        name: "LongLeftRightArrow",
        value: "\u{27F7}",
        legacy: false,
    },
    NamedRef {
        name: "LongRightArrow",
        value: "\u{27F6}",
        legacy: false,
    },
    NamedRef {
        name: "Longleftarrow",
        value: "\u{27F8}",
        legacy: false,
    },
    NamedRef {
        name: "Longleftrightarrow",
        value: "\u{27FA}",
        legacy: false,
    },
    NamedRef {
        name: "Longrightarrow",
        value: "\u{27F9}",
        legacy: false,
    },
    NamedRef {
        name: "Lopf",
        value: "\u{1D543}",
        legacy: false,
    },
    NamedRef {
        name: "LowerLeftArrow",
        value: "\u{2199}",
        legacy: false,
    },
    NamedRef {
        name: "LowerRightArrow",
        value: "\u{2198}",
        legacy: false,
    },
    NamedRef {
        name: "Lscr",
        value: "\u{2112}",
        legacy: false,
    },
    NamedRef {
        name: "Lsh",
        value: "\u{21B0}",
        legacy: false,
    },
    NamedRef {
        name: "Lstrok",
        value: "\u{141}",
        legacy: false,
    },
    NamedRef {
        name: "Lt",
        value: "\u{226A}",
        legacy: false,
    },
    NamedRef {
        name: "Map",
        value: "\u{2905}",
        legacy: false,
    },
    NamedRef {
        name: "Mcy",
        value: "\u{41C}",
        legacy: false,
    },
    NamedRef {
        name: "MediumSpace",
        value: "\u{205F}",
        legacy: false,
    },
    NamedRef {
        name: "Mellintrf",
        value: "\u{2133}",
        legacy: false,
    },
    NamedRef {
        name: "Mfr",
        value: "\u{1D510}",
        legacy: false,
    },
    NamedRef {
        name: "MinusPlus",
        value: "\u{2213}",
        legacy: false,
    },
    NamedRef {
        name: "Mopf",
        value: "\u{1D544}",
        legacy: false,
    },
    NamedRef {
        name: "Mscr",
        value: "\u{2133}",
        legacy: false,
    },
    NamedRef {
        name: "Mu",
        value: "\u{39C}",
        legacy: false,
    },
    NamedRef {
        name: "NJcy",
        value: "\u{40A}",
        legacy: false,
    },
    NamedRef {
        name: "Nacute",
        value: "\u{143}",
        legacy: false,
    },
    NamedRef {
        name: "Ncaron",
        value: "\u{147}",
        legacy: false,
    },
    NamedRef {
        name: "Ncedil",
        value: "\u{145}",
        legacy: false,
    },
    NamedRef {
        name: "Ncy",
        value: "\u{41D}",
        legacy: false,
    },
    NamedRef {
        name: "NegativeMediumSpace",
        value: "\u{200B}",
        legacy: false,
    },
    NamedRef {
        name: "NegativeThickSpace",
        value: "\u{200B}",
        legacy: false,
    },
    NamedRef {
        name: "NegativeThinSpace",
        value: "\u{200B}",
        legacy: false,
    },
    NamedRef {
        name: "NegativeVeryThinSpace",
        value: "\u{200B}",
        legacy: false,
    },
    NamedRef {
        name: "NestedGreaterGreater",
        value: "\u{226B}",
        legacy: false,
    },
    NamedRef {
        name: "NestedLessLess",
        value: "\u{226A}",
        legacy: false,
    },
    NamedRef {
        name: "NewLine",
        value: "\u{A}",
        legacy: false,
    },
    NamedRef {
        name: "Nfr",
        value: "\u{1D511}",
        legacy: false,
    },
    NamedRef {
        name: "NoBreak",
        value: "\u{2060}",
        legacy: false,
    },
    NamedRef {
        name: "NonBreakingSpace",
        value: "\u{A0}",
        legacy: false,
    },
    NamedRef {
        name: "Nopf",
        value: "\u{2115}",
        legacy: false,
    },
    NamedRef {
        name: "Not",
        value: "\u{2AEC}",
        legacy: false,
    },
    NamedRef {
        name: "NotCongruent",
        value: "\u{2262}",
        legacy: false,
    },
    NamedRef {
        name: "NotCupCap",
        value: "\u{226D}",
        legacy: false,
    },
    NamedRef {
        name: "NotDoubleVerticalBar",
        value: "\u{2226}",
        legacy: false,
    },
    NamedRef {
        name: "NotElement",
        value: "\u{2209}",
        legacy: false,
    },
    NamedRef {
        name: "NotEqual",
        value: "\u{2260}",
        legacy: false,
    },
    NamedRef {
        name: "NotEqualTilde",
        value: "\u{2242}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotExists",
        value: "\u{2204}",
        legacy: false,
    },
    NamedRef {
        name: "NotGreater",
        value: "\u{226F}",
        legacy: false,
    },
    NamedRef {
        name: "NotGreaterEqual",
        value: "\u{2271}",
        legacy: false,
    },
    NamedRef {
        name: "NotGreaterFullEqual",
        value: "\u{2267}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotGreaterGreater",
        value: "\u{226B}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotGreaterLess",
        value: "\u{2279}",
        legacy: false,
    },
    NamedRef {
        name: "NotGreaterSlantEqual",
        value: "\u{2A7E}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotGreaterTilde",
        value: "\u{2275}",
        legacy: false,
    },
    NamedRef {
        name: "NotHumpDownHump",
        value: "\u{224E}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotHumpEqual",
        value: "\u{224F}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotLeftTriangle",
        value: "\u{22EA}",
        legacy: false,
    },
    NamedRef {
        name: "NotLeftTriangleBar",
        value: "\u{29CF}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotLeftTriangleEqual",
        value: "\u{22EC}",
        legacy: false,
    },
    NamedRef {
        name: "NotLess",
        value: "\u{226E}",
        legacy: false,
    },
    NamedRef {
        name: "NotLessEqual",
        value: "\u{2270}",
        legacy: false,
    },
    NamedRef {
        name: "NotLessGreater",
        value: "\u{2278}",
        legacy: false,
    },
    NamedRef {
        name: "NotLessLess",
        value: "\u{226A}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotLessSlantEqual",
        value: "\u{2A7D}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotLessTilde",
        value: "\u{2274}",
        legacy: false,
    },
    NamedRef {
        name: "NotNestedGreaterGreater",
        value: "\u{2AA2}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotNestedLessLess",
        value: "\u{2AA1}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotPrecedes",
        value: "\u{2280}",
        legacy: false,
    },
    NamedRef {
        name: "NotPrecedesEqual",
        value: "\u{2AAF}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotPrecedesSlantEqual",
        value: "\u{22E0}",
        legacy: false,
    },
    NamedRef {
        name: "NotReverseElement",
        value: "\u{220C}",
        legacy: false,
    },
    NamedRef {
        name: "NotRightTriangle",
        value: "\u{22EB}",
        legacy: false,
    },
    NamedRef {
        name: "NotRightTriangleBar",
        value: "\u{29D0}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotRightTriangleEqual",
        value: "\u{22ED}",
        legacy: false,
    },
    NamedRef {
        name: "NotSquareSubset",
        value: "\u{228F}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotSquareSubsetEqual",
        value: "\u{22E2}",
        legacy: false,
    },
    NamedRef {
        name: "NotSquareSuperset",
        value: "\u{2290}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotSquareSupersetEqual",
        value: "\u{22E3}",
        legacy: false,
    },
    NamedRef {
        name: "NotSubset",
        value: "\u{2282}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "NotSubsetEqual",
        value: "\u{2288}",
        legacy: false,
    },
    NamedRef {
        name: "NotSucceeds",
        value: "\u{2281}",
        legacy: false,
    },
    NamedRef {
        name: "NotSucceedsEqual",
        value: "\u{2AB0}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotSucceedsSlantEqual",
        value: "\u{22E1}",
        legacy: false,
    },
    NamedRef {
        name: "NotSucceedsTilde",
        value: "\u{227F}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "NotSuperset",
        value: "\u{2283}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "NotSupersetEqual",
        value: "\u{2289}",
        legacy: false,
    },
    NamedRef {
        name: "NotTilde",
        value: "\u{2241}",
        legacy: false,
    },
    NamedRef {
        name: "NotTildeEqual",
        value: "\u{2244}",
        legacy: false,
    },
    NamedRef {
        name: "NotTildeFullEqual",
        value: "\u{2247}",
        legacy: false,
    },
    NamedRef {
        name: "NotTildeTilde",
        value: "\u{2249}",
        legacy: false,
    },
    NamedRef {
        name: "NotVerticalBar",
        value: "\u{2224}",
        legacy: false,
    },
    NamedRef {
        name: "Nscr",
        value: "\u{1D4A9}",
        legacy: false,
    },
    NamedRef {
        name: "Ntilde",
        value: "\u{D1}",
        legacy: true,
    },
    NamedRef {
        name: "Nu",
        value: "\u{39D}",
        legacy: false,
    },
    NamedRef {
        name: "OElig",
        value: "\u{152}",
        legacy: false,
    },
    NamedRef {
        name: "Oacute",
        value: "\u{D3}",
        legacy: true,
    },
    NamedRef {
        name: "Ocirc",
        value: "\u{D4}",
        legacy: true,
    },
    NamedRef {
        name: "Ocy",
        value: "\u{41E}",
        legacy: false,
    },
    NamedRef {
        name: "Odblac",
        value: "\u{150}",
        legacy: false,
    },
    NamedRef {
        name: "Ofr",
        value: "\u{1D512}",
        legacy: false,
    },
    NamedRef {
        name: "Ograve",
        value: "\u{D2}",
        legacy: true,
    },
    NamedRef {
        name: "Omacr",
        value: "\u{14C}",
        legacy: false,
    },
    NamedRef {
        name: "Omega",
        value: "\u{3A9}",
        legacy: false,
    },
    NamedRef {
        name: "Omicron",
        value: "\u{39F}",
        legacy: false,
    },
    NamedRef {
        name: "Oopf",
        value: "\u{1D546}",
        legacy: false,
    },
    NamedRef {
        name: "OpenCurlyDoubleQuote",
        value: "\u{201C}",
        legacy: false,
    },
    NamedRef {
        name: "OpenCurlyQuote",
        value: "\u{2018}",
        legacy: false,
    },
    NamedRef {
        name: "Or",
        value: "\u{2A54}",
        legacy: false,
    },
    NamedRef {
        name: "Oscr",
        value: "\u{1D4AA}",
        legacy: false,
    },
    NamedRef {
        name: "Oslash",
        value: "\u{D8}",
        legacy: true,
    },
    NamedRef {
        name: "Otilde",
        value: "\u{D5}",
        legacy: true,
    },
    NamedRef {
        name: "Otimes",
        value: "\u{2A37}",
        legacy: false,
    },
    NamedRef {
        name: "Ouml",
        value: "\u{D6}",
        legacy: true,
    },
    NamedRef {
        name: "OverBar",
        value: "\u{203E}",
        legacy: false,
    },
    NamedRef {
        name: "OverBrace",
        value: "\u{23DE}",
        legacy: false,
    },
    NamedRef {
        name: "OverBracket",
        value: "\u{23B4}",
        legacy: false,
    },
    NamedRef {
        name: "OverParenthesis",
        value: "\u{23DC}",
        legacy: false,
    },
    NamedRef {
        name: "PartialD",
        value: "\u{2202}",
        legacy: false,
    },
    NamedRef {
        name: "Pcy",
        value: "\u{41F}",
        legacy: false,
    },
    NamedRef {
        name: "Pfr",
        value: "\u{1D513}",
        legacy: false,
    },
    NamedRef {
        name: "Phi",
        value: "\u{3A6}",
        legacy: false,
    },
    NamedRef {
        name: "Pi",
        value: "\u{3A0}",
        legacy: false,
    },
    NamedRef {
        name: "PlusMinus",
        value: "\u{B1}",
        legacy: false,
    },
    NamedRef {
        name: "Poincareplane",
        value: "\u{210C}",
        legacy: false,
    },
    NamedRef {
        name: "Popf",
        value: "\u{2119}",
        legacy: false,
    },
    NamedRef {
        name: "Pr",
        value: "\u{2ABB}",
        legacy: false,
    },
    NamedRef {
        name: "Precedes",
        value: "\u{227A}",
        legacy: false,
    },
    NamedRef {
        name: "PrecedesEqual",
        value: "\u{2AAF}",
        legacy: false,
    },
    NamedRef {
        name: "PrecedesSlantEqual",
        value: "\u{227C}",
        legacy: false,
    },
    NamedRef {
        name: "PrecedesTilde",
        value: "\u{227E}",
        legacy: false,
    },
    NamedRef {
        name: "Prime",
        value: "\u{2033}",
        legacy: false,
    },
    NamedRef {
        name: "Product",
        value: "\u{220F}",
        legacy: false,
    },
    NamedRef {
        name: "Proportion",
        value: "\u{2237}",
        legacy: false,
    },
    NamedRef {
        name: "Proportional",
        value: "\u{221D}",
        legacy: false,
    },
    NamedRef {
        name: "Pscr",
        value: "\u{1D4AB}",
        legacy: false,
    },
    NamedRef {
        name: "Psi",
        value: "\u{3A8}",
        legacy: false,
    },
    NamedRef {
        name: "QUOT",
        value: "\u{22}",
        legacy: true,
    },
    NamedRef {
        name: "Qfr",
        value: "\u{1D514}",
        legacy: false,
    },
    NamedRef {
        name: "Qopf",
        value: "\u{211A}",
        legacy: false,
    },
    NamedRef {
        name: "Qscr",
        value: "\u{1D4AC}",
        legacy: false,
    },
    NamedRef {
        name: "RBarr",
        value: "\u{2910}",
        legacy: false,
    },
    NamedRef {
        name: "REG",
        value: "\u{AE}",
        legacy: true,
    },
    NamedRef {
        name: "Racute",
        value: "\u{154}",
        legacy: false,
    },
    NamedRef {
        name: "Rang",
        value: "\u{27EB}",
        legacy: false,
    },
    NamedRef {
        name: "Rarr",
        value: "\u{21A0}",
        legacy: false,
    },
    NamedRef {
        name: "Rarrtl",
        value: "\u{2916}",
        legacy: false,
    },
    NamedRef {
        name: "Rcaron",
        value: "\u{158}",
        legacy: false,
    },
    NamedRef {
        name: "Rcedil",
        value: "\u{156}",
        legacy: false,
    },
    NamedRef {
        name: "Rcy",
        value: "\u{420}",
        legacy: false,
    },
    NamedRef {
        name: "Re",
        value: "\u{211C}",
        legacy: false,
    },
    NamedRef {
        name: "ReverseElement",
        value: "\u{220B}",
        legacy: false,
    },
    NamedRef {
        name: "ReverseEquilibrium",
        value: "\u{21CB}",
        legacy: false,
    },
    NamedRef {
        name: "ReverseUpEquilibrium",
        value: "\u{296F}",
        legacy: false,
    },
    NamedRef {
        name: "Rfr",
        value: "\u{211C}",
        legacy: false,
    },
    NamedRef {
        name: "Rho",
        value: "\u{3A1}",
        legacy: false,
    },
    NamedRef {
        name: "RightAngleBracket",
        value: "\u{27E9}",
        legacy: false,
    },
    NamedRef {
        name: "RightArrow",
        value: "\u{2192}",
        legacy: false,
    },
    NamedRef {
        name: "RightArrowBar",
        value: "\u{21E5}",
        legacy: false,
    },
    NamedRef {
        name: "RightArrowLeftArrow",
        value: "\u{21C4}",
        legacy: false,
    },
    NamedRef {
        name: "RightCeiling",
        value: "\u{2309}",
        legacy: false,
    },
    NamedRef {
        name: "RightDoubleBracket",
        value: "\u{27E7}",
        legacy: false,
    },
    NamedRef {
        name: "RightDownTeeVector",
        value: "\u{295D}",
        legacy: false,
    },
    NamedRef {
        name: "RightDownVector",
        value: "\u{21C2}",
        legacy: false,
    },
    NamedRef {
        name: "RightDownVectorBar",
        value: "\u{2955}",
        legacy: false,
    },
    NamedRef {
        name: "RightFloor",
        value: "\u{230B}",
        legacy: false,
    },
    NamedRef {
        name: "RightTee",
        value: "\u{22A2}",
        legacy: false,
    },
    NamedRef {
        name: "RightTeeArrow",
        value: "\u{21A6}",
        legacy: false,
    },
    NamedRef {
        name: "RightTeeVector",
        value: "\u{295B}",
        legacy: false,
    },
    NamedRef {
        name: "RightTriangle",
        value: "\u{22B3}",
        legacy: false,
    },
    NamedRef {
        name: "RightTriangleBar",
        value: "\u{29D0}",
        legacy: false,
    },
    NamedRef {
        name: "RightTriangleEqual",
        value: "\u{22B5}",
        legacy: false,
    },
    NamedRef {
        name: "RightUpDownVector",
        value: "\u{294F}",
        legacy: false,
    },
    NamedRef {
        name: "RightUpTeeVector",
        value: "\u{295C}",
        legacy: false,
    },
    NamedRef {
        name: "RightUpVector",
        value: "\u{21BE}",
        legacy: false,
    },
    NamedRef {
        name: "RightUpVectorBar",
        value: "\u{2954}",
        legacy: false,
    },
    NamedRef {
        name: "RightVector",
        value: "\u{21C0}",
        legacy: false,
    },
    NamedRef {
        name: "RightVectorBar",
        value: "\u{2953}",
        legacy: false,
    },
    NamedRef {
        name: "Rightarrow",
        value: "\u{21D2}",
        legacy: false,
    },
    NamedRef {
        name: "Ropf",
        value: "\u{211D}",
        legacy: false,
    },
    NamedRef {
        name: "RoundImplies",
        value: "\u{2970}",
        legacy: false,
    },
    NamedRef {
        name: "Rrightarrow",
        value: "\u{21DB}",
        legacy: false,
    },
    NamedRef {
        name: "Rscr",
        value: "\u{211B}",
        legacy: false,
    },
    NamedRef {
        name: "Rsh",
        value: "\u{21B1}",
        legacy: false,
    },
    NamedRef {
        name: "RuleDelayed",
        value: "\u{29F4}",
        legacy: false,
    },
    NamedRef {
        name: "SHCHcy",
        value: "\u{429}",
        legacy: false,
    },
    NamedRef {
        name: "SHcy",
        value: "\u{428}",
        legacy: false,
    },
    NamedRef {
        name: "SOFTcy",
        value: "\u{42C}",
        legacy: false,
    },
    NamedRef {
        name: "Sacute",
        value: "\u{15A}",
        legacy: false,
    },
    NamedRef {
        name: "Sc",
        value: "\u{2ABC}",
        legacy: false,
    },
    NamedRef {
        name: "Scaron",
        value: "\u{160}",
        legacy: false,
    },
    NamedRef {
        name: "Scedil",
        value: "\u{15E}",
        legacy: false,
    },
    NamedRef {
        name: "Scirc",
        value: "\u{15C}",
        legacy: false,
    },
    NamedRef {
        name: "Scy",
        value: "\u{421}",
        legacy: false,
    },
    NamedRef {
        name: "Sfr",
        value: "\u{1D516}",
        legacy: false,
    },
    NamedRef {
        name: "ShortDownArrow",
        value: "\u{2193}",
        legacy: false,
    },
    NamedRef {
        name: "ShortLeftArrow",
        value: "\u{2190}",
        legacy: false,
    },
    NamedRef {
        name: "ShortRightArrow",
        value: "\u{2192}",
        legacy: false,
    },
    NamedRef {
        name: "ShortUpArrow",
        value: "\u{2191}",
        legacy: false,
    },
    NamedRef {
        name: "Sigma",
        value: "\u{3A3}",
        legacy: false,
    },
    NamedRef {
        name: "SmallCircle",
        value: "\u{2218}",
        legacy: false,
    },
    NamedRef {
        name: "Sopf",
        value: "\u{1D54A}",
        legacy: false,
    },
    NamedRef {
        name: "Sqrt",
        value: "\u{221A}",
        legacy: false,
    },
    NamedRef {
        name: "Square",
        value: "\u{25A1}",
        legacy: false,
    },
    NamedRef {
        name: "SquareIntersection",
        value: "\u{2293}",
        legacy: false,
    },
    NamedRef {
        name: "SquareSubset",
        value: "\u{228F}",
        legacy: false,
    },
    NamedRef {
        name: "SquareSubsetEqual",
        value: "\u{2291}",
        legacy: false,
    },
    NamedRef {
        name: "SquareSuperset",
        value: "\u{2290}",
        legacy: false,
    },
    NamedRef {
        name: "SquareSupersetEqual",
        value: "\u{2292}",
        legacy: false,
    },
    NamedRef {
        name: "SquareUnion",
        value: "\u{2294}",
        legacy: false,
    },
    NamedRef {
        name: "Sscr",
        value: "\u{1D4AE}",
        legacy: false,
    },
    NamedRef {
        name: "Star",
        value: "\u{22C6}",
        legacy: false,
    },
    NamedRef {
        name: "Sub",
        value: "\u{22D0}",
        legacy: false,
    },
    NamedRef {
        name: "Subset",
        value: "\u{22D0}",
        legacy: false,
    },
    NamedRef {
        name: "SubsetEqual",
        value: "\u{2286}",
        legacy: false,
    },
    NamedRef {
        name: "Succeeds",
        value: "\u{227B}",
        legacy: false,
    },
    NamedRef {
        name: "SucceedsEqual",
        value: "\u{2AB0}",
        legacy: false,
    },
    NamedRef {
        name: "SucceedsSlantEqual",
        value: "\u{227D}",
        legacy: false,
    },
    NamedRef {
        name: "SucceedsTilde",
        value: "\u{227F}",
        legacy: false,
    },
    NamedRef {
        name: "SuchThat",
        value: "\u{220B}",
        legacy: false,
    },
    NamedRef {
        name: "Sum",
        value: "\u{2211}",
        legacy: false,
    },
    NamedRef {
        name: "Sup",
        value: "\u{22D1}",
        legacy: false,
    },
    NamedRef {
        name: "Superset",
        value: "\u{2283}",
        legacy: false,
    },
    NamedRef {
        name: "SupersetEqual",
        value: "\u{2287}",
        legacy: false,
    },
    NamedRef {
        name: "Supset",
        value: "\u{22D1}",
        legacy: false,
    },
    NamedRef {
        name: "THORN",
        value: "\u{DE}",
        legacy: true,
    },
    NamedRef {
        name: "TRADE",
        value: "\u{2122}",
        legacy: false,
    },
    NamedRef {
        name: "TSHcy",
        value: "\u{40B}",
        legacy: false,
    },
    NamedRef {
        name: "TScy",
        value: "\u{426}",
        legacy: false,
    },
    NamedRef {
        name: "Tab",
        value: "\u{9}",
        legacy: false,
    },
    NamedRef {
        name: "Tau",
        value: "\u{3A4}",
        legacy: false,
    },
    NamedRef {
        name: "Tcaron",
        value: "\u{164}",
        legacy: false,
    },
    NamedRef {
        name: "Tcedil",
        value: "\u{162}",
        legacy: false,
    },
    NamedRef {
        name: "Tcy",
        value: "\u{422}",
        legacy: false,
    },
    NamedRef {
        name: "Tfr",
        value: "\u{1D517}",
        legacy: false,
    },
    NamedRef {
        name: "Therefore",
        value: "\u{2234}",
        legacy: false,
    },
    NamedRef {
        name: "Theta",
        value: "\u{398}",
        legacy: false,
    },
    NamedRef {
        name: "ThickSpace",
        value: "\u{205F}\u{200A}",
        legacy: false,
    },
    NamedRef {
        name: "ThinSpace",
        value: "\u{2009}",
        legacy: false,
    },
    NamedRef {
        name: "Tilde",
        value: "\u{223C}",
        legacy: false,
    },
    NamedRef {
        name: "TildeEqual",
        value: "\u{2243}",
        legacy: false,
    },
    NamedRef {
        name: "TildeFullEqual",
        value: "\u{2245}",
        legacy: false,
    },
    NamedRef {
        name: "TildeTilde",
        value: "\u{2248}",
        legacy: false,
    },
    NamedRef {
        name: "Topf",
        value: "\u{1D54B}",
        legacy: false,
    },
    NamedRef {
        name: "TripleDot",
        value: "\u{20DB}",
        legacy: false,
    },
    NamedRef {
        name: "Tscr",
        value: "\u{1D4AF}",
        legacy: false,
    },
    NamedRef {
        name: "Tstrok",
        value: "\u{166}",
        legacy: false,
    },
    NamedRef {
        name: "Uacute",
        value: "\u{DA}",
        legacy: true,
    },
    NamedRef {
        name: "Uarr",
        value: "\u{219F}",
        legacy: false,
    },
    NamedRef {
        name: "Uarrocir",
        value: "\u{2949}",
        legacy: false,
    },
    NamedRef {
        name: "Ubrcy",
        value: "\u{40E}",
        legacy: false,
    },
    NamedRef {
        name: "Ubreve",
        value: "\u{16C}",
        legacy: false,
    },
    NamedRef {
        name: "Ucirc",
        value: "\u{DB}",
        legacy: true,
    },
    NamedRef {
        name: "Ucy",
        value: "\u{423}",
        legacy: false,
    },
    NamedRef {
        name: "Udblac",
        value: "\u{170}",
        legacy: false,
    },
    NamedRef {
        name: "Ufr",
        value: "\u{1D518}",
        legacy: false,
    },
    NamedRef {
        name: "Ugrave",
        value: "\u{D9}",
        legacy: true,
    },
    NamedRef {
        name: "Umacr",
        value: "\u{16A}",
        legacy: false,
    },
    NamedRef {
        name: "UnderBar",
        value: "\u{5F}",
        legacy: false,
    },
    NamedRef {
        name: "UnderBrace",
        value: "\u{23DF}",
        legacy: false,
    },
    NamedRef {
        name: "UnderBracket",
        value: "\u{23B5}",
        legacy: false,
    },
    NamedRef {
        name: "UnderParenthesis",
        value: "\u{23DD}",
        legacy: false,
    },
    NamedRef {
        name: "Union",
        value: "\u{22C3}",
        legacy: false,
    },
    NamedRef {
        name: "UnionPlus",
        value: "\u{228E}",
        legacy: false,
    },
    NamedRef {
        name: "Uogon",
        value: "\u{172}",
        legacy: false,
    },
    NamedRef {
        name: "Uopf",
        value: "\u{1D54C}",
        legacy: false,
    },
    NamedRef {
        name: "UpArrow",
        value: "\u{2191}",
        legacy: false,
    },
    NamedRef {
        name: "UpArrowBar",
        value: "\u{2912}",
        legacy: false,
    },
    NamedRef {
        name: "UpArrowDownArrow",
        value: "\u{21C5}",
        legacy: false,
    },
    NamedRef {
        name: "UpDownArrow",
        value: "\u{2195}",
        legacy: false,
    },
    NamedRef {
        name: "UpEquilibrium",
        value: "\u{296E}",
        legacy: false,
    },
    NamedRef {
        name: "UpTee",
        value: "\u{22A5}",
        legacy: false,
    },
    NamedRef {
        name: "UpTeeArrow",
        value: "\u{21A5}",
        legacy: false,
    },
    NamedRef {
        name: "Uparrow",
        value: "\u{21D1}",
        legacy: false,
    },
    NamedRef {
        name: "Updownarrow",
        value: "\u{21D5}",
        legacy: false,
    },
    NamedRef {
        name: "UpperLeftArrow",
        value: "\u{2196}",
        legacy: false,
    },
    NamedRef {
        name: "UpperRightArrow",
        value: "\u{2197}",
        legacy: false,
    },
    NamedRef {
        name: "Upsi",
        value: "\u{3D2}",
        legacy: false,
    },
    NamedRef {
        name: "Upsilon",
        value: "\u{3A5}",
        legacy: false,
    },
    NamedRef {
        name: "Uring",
        value: "\u{16E}",
        legacy: false,
    },
    NamedRef {
        name: "Uscr",
        value: "\u{1D4B0}",
        legacy: false,
    },
    NamedRef {
        name: "Utilde",
        value: "\u{168}",
        legacy: false,
    },
    NamedRef {
        name: "Uuml",
        value: "\u{DC}",
        legacy: true,
    },
    NamedRef {
        name: "VDash",
        value: "\u{22AB}",
        legacy: false,
    },
    NamedRef {
        name: "Vbar",
        value: "\u{2AEB}",
        legacy: false,
    },
    NamedRef {
        name: "Vcy",
        value: "\u{412}",
        legacy: false,
    },
    NamedRef {
        name: "Vdash",
        value: "\u{22A9}",
        legacy: false,
    },
    NamedRef {
        name: "Vdashl",
        value: "\u{2AE6}",
        legacy: false,
    },
    NamedRef {
        name: "Vee",
        value: "\u{22C1}",
        legacy: false,
    },
    NamedRef {
        name: "Verbar",
        value: "\u{2016}",
        legacy: false,
    },
    NamedRef {
        name: "Vert",
        value: "\u{2016}",
        legacy: false,
    },
    NamedRef {
        name: "VerticalBar",
        value: "\u{2223}",
        legacy: false,
    },
    NamedRef {
        name: "VerticalLine",
        value: "\u{7C}",
        legacy: false,
    },
    NamedRef {
        name: "VerticalSeparator",
        value: "\u{2758}",
        legacy: false,
    },
    NamedRef {
        name: "VerticalTilde",
        value: "\u{2240}",
        legacy: false,
    },
    NamedRef {
        name: "VeryThinSpace",
        value: "\u{200A}",
        legacy: false,
    },
    NamedRef {
        name: "Vfr",
        value: "\u{1D519}",
        legacy: false,
    },
    NamedRef {
        name: "Vopf",
        value: "\u{1D54D}",
        legacy: false,
    },
    NamedRef {
        name: "Vscr",
        value: "\u{1D4B1}",
        legacy: false,
    },
    NamedRef {
        name: "Vvdash",
        value: "\u{22AA}",
        legacy: false,
    },
    NamedRef {
        name: "Wcirc",
        value: "\u{174}",
        legacy: false,
    },
    NamedRef {
        name: "Wedge",
        value: "\u{22C0}",
        legacy: false,
    },
    NamedRef {
        name: "Wfr",
        value: "\u{1D51A}",
        legacy: false,
    },
    NamedRef {
        name: "Wopf",
        value: "\u{1D54E}",
        legacy: false,
    },
    NamedRef {
        name: "Wscr",
        value: "\u{1D4B2}",
        legacy: false,
    },
    NamedRef {
        name: "Xfr",
        value: "\u{1D51B}",
        legacy: false,
    },
    NamedRef {
        name: "Xi",
        value: "\u{39E}",
        legacy: false,
    },
    NamedRef {
        name: "Xopf",
        value: "\u{1D54F}",
        legacy: false,
    },
    NamedRef {
        name: "Xscr",
        value: "\u{1D4B3}",
        legacy: false,
    },
    NamedRef {
        name: "YAcy",
        value: "\u{42F}",
        legacy: false,
    },
    NamedRef {
        name: "YIcy",
        value: "\u{407}",
        legacy: false,
    },
    NamedRef {
        name: "YUcy",
        value: "\u{42E}",
        legacy: false,
    },
    NamedRef {
        name: "Yacute",
        value: "\u{DD}",
        legacy: true,
    },
    NamedRef {
        name: "Ycirc",
        value: "\u{176}",
        legacy: false,
    },
    NamedRef {
        name: "Ycy",
        value: "\u{42B}",
        legacy: false,
    },
    NamedRef {
        name: "Yfr",
        value: "\u{1D51C}",
        legacy: false,
    },
    NamedRef {
        name: "Yopf",
        value: "\u{1D550}",
        legacy: false,
    },
    NamedRef {
        name: "Yscr",
        value: "\u{1D4B4}",
        legacy: false,
    },
    NamedRef {
        name: "Yuml",
        value: "\u{178}",
        legacy: false,
    },
    NamedRef {
        name: "ZHcy",
        value: "\u{416}",
        legacy: false,
    },
    NamedRef {
        name: "Zacute",
        value: "\u{179}",
        legacy: false,
    },
    NamedRef {
        name: "Zcaron",
        value: "\u{17D}",
        legacy: false,
    },
    NamedRef {
        name: "Zcy",
        value: "\u{417}",
        legacy: false,
    },
    NamedRef {
        name: "Zdot",
        value: "\u{17B}",
        legacy: false,
    },
    NamedRef {
        name: "ZeroWidthSpace",
        value: "\u{200B}",
        legacy: false,
    },
    NamedRef {
        name: "Zeta",
        value: "\u{396}",
        legacy: false,
    },
    NamedRef {
        name: "Zfr",
        value: "\u{2128}",
        legacy: false,
    },
    NamedRef {
        name: "Zopf",
        value: "\u{2124}",
        legacy: false,
    },
    NamedRef {
        name: "Zscr",
        value: "\u{1D4B5}",
        legacy: false,
    },
    NamedRef {
        name: "aacute",
        value: "\u{E1}",
        legacy: true,
    },
    NamedRef {
        name: "abreve",
        value: "\u{103}",
        legacy: false,
    },
    NamedRef {
        name: "ac",
        value: "\u{223E}",
        legacy: false,
    },
    NamedRef {
        name: "acE",
        value: "\u{223E}\u{333}",
        legacy: false,
    },
    NamedRef {
        name: "acd",
        value: "\u{223F}",
        legacy: false,
    },
    NamedRef {
        name: "acirc",
        value: "\u{E2}",
        legacy: true,
    },
    NamedRef {
        name: "acute",
        value: "\u{B4}",
        legacy: true,
    },
    NamedRef {
        name: "acy",
        value: "\u{430}",
        legacy: false,
    },
    NamedRef {
        name: "aelig",
        value: "\u{E6}",
        legacy: true,
    },
    NamedRef {
        name: "af",
        value: "\u{2061}",
        legacy: false,
    },
    NamedRef {
        name: "afr",
        value: "\u{1D51E}",
        legacy: false,
    },
    NamedRef {
        name: "agrave",
        value: "\u{E0}",
        legacy: true,
    },
    NamedRef {
        name: "alefsym",
        value: "\u{2135}",
        legacy: false,
    },
    NamedRef {
        name: "aleph",
        value: "\u{2135}",
        legacy: false,
    },
    NamedRef {
        name: "alpha",
        value: "\u{3B1}",
        legacy: false,
    },
    NamedRef {
        name: "amacr",
        value: "\u{101}",
        legacy: false,
    },
    NamedRef {
        name: "amalg",
        value: "\u{2A3F}",
        legacy: false,
    },
    NamedRef {
        name: "amp",
        value: "\u{26}",
        legacy: true,
    },
    NamedRef {
        name: "and",
        value: "\u{2227}",
        legacy: false,
    },
    NamedRef {
        name: "andand",
        value: "\u{2A55}",
        legacy: false,
    },
    NamedRef {
        name: "andd",
        value: "\u{2A5C}",
        legacy: false,
    },
    NamedRef {
        name: "andslope",
        value: "\u{2A58}",
        legacy: false,
    },
    NamedRef {
        name: "andv",
        value: "\u{2A5A}",
        legacy: false,
    },
    NamedRef {
        name: "ang",
        value: "\u{2220}",
        legacy: false,
    },
    NamedRef {
        name: "ange",
        value: "\u{29A4}",
        legacy: false,
    },
    NamedRef {
        name: "angle",
        value: "\u{2220}",
        legacy: false,
    },
    NamedRef {
        name: "angmsd",
        value: "\u{2221}",
        legacy: false,
    },
    NamedRef {
        name: "angmsdaa",
        value: "\u{29A8}",
        legacy: false,
    },
    NamedRef {
        name: "angmsdab",
        value: "\u{29A9}",
        legacy: false,
    },
    NamedRef {
        name: "angmsdac",
        value: "\u{29AA}",
        legacy: false,
    },
    NamedRef {
        name: "angmsdad",
        value: "\u{29AB}",
        legacy: false,
    },
    NamedRef {
        name: "angmsdae",
        value: "\u{29AC}",
        legacy: false,
    },
    NamedRef {
        name: "angmsdaf",
        value: "\u{29AD}",
        legacy: false,
    },
    NamedRef {
        name: "angmsdag",
        value: "\u{29AE}",
        legacy: false,
    },
    NamedRef {
        name: "angmsdah",
        value: "\u{29AF}",
        legacy: false,
    },
    NamedRef {
        name: "angrt",
        value: "\u{221F}",
        legacy: false,
    },
    NamedRef {
        name: "angrtvb",
        value: "\u{22BE}",
        legacy: false,
    },
    NamedRef {
        name: "angrtvbd",
        value: "\u{299D}",
        legacy: false,
    },
    NamedRef {
        name: "angsph",
        value: "\u{2222}",
        legacy: false,
    },
    NamedRef {
        name: "angst",
        value: "\u{C5}",
        legacy: false,
    },
    NamedRef {
        name: "angzarr",
        value: "\u{237C}",
        legacy: false,
    },
    NamedRef {
        name: "aogon",
        value: "\u{105}",
        legacy: false,
    },
    NamedRef {
        name: "aopf",
        value: "\u{1D552}",
        legacy: false,
    },
    NamedRef {
        name: "ap",
        value: "\u{2248}",
        legacy: false,
    },
    NamedRef {
        name: "apE",
        value: "\u{2A70}",
        legacy: false,
    },
    NamedRef {
        name: "apacir",
        value: "\u{2A6F}",
        legacy: false,
    },
    NamedRef {
        name: "ape",
        value: "\u{224A}",
        legacy: false,
    },
    NamedRef {
        name: "apid",
        value: "\u{224B}",
        legacy: false,
    },
    NamedRef {
        name: "apos",
        value: "\u{27}",
        legacy: false,
    },
    NamedRef {
        name: "approx",
        value: "\u{2248}",
        legacy: false,
    },
    NamedRef {
        name: "approxeq",
        value: "\u{224A}",
        legacy: false,
    },
    NamedRef {
        name: "aring",
        value: "\u{E5}",
        legacy: true,
    },
    NamedRef {
        name: "ascr",
        value: "\u{1D4B6}",
        legacy: false,
    },
    NamedRef {
        name: "ast",
        value: "\u{2A}",
        legacy: false,
    },
    NamedRef {
        name: "asymp",
        value: "\u{2248}",
        legacy: false,
    },
    NamedRef {
        name: "asympeq",
        value: "\u{224D}",
        legacy: false,
    },
    NamedRef {
        name: "atilde",
        value: "\u{E3}",
        legacy: true,
    },
    NamedRef {
        name: "auml",
        value: "\u{E4}",
        legacy: true,
    },
    NamedRef {
        name: "awconint",
        value: "\u{2233}",
        legacy: false,
    },
    NamedRef {
        name: "awint",
        value: "\u{2A11}",
        legacy: false,
    },
    NamedRef {
        name: "bNot",
        value: "\u{2AED}",
        legacy: false,
    },
    NamedRef {
        name: "backcong",
        value: "\u{224C}",
        legacy: false,
    },
    NamedRef {
        name: "backepsilon",
        value: "\u{3F6}",
        legacy: false,
    },
    NamedRef {
        name: "backprime",
        value: "\u{2035}",
        legacy: false,
    },
    NamedRef {
        name: "backsim",
        value: "\u{223D}",
        legacy: false,
    },
    NamedRef {
        name: "backsimeq",
        value: "\u{22CD}",
        legacy: false,
    },
    NamedRef {
        name: "barvee",
        value: "\u{22BD}",
        legacy: false,
    },
    NamedRef {
        name: "barwed",
        value: "\u{2305}",
        legacy: false,
    },
    NamedRef {
        name: "barwedge",
        value: "\u{2305}",
        legacy: false,
    },
    NamedRef {
        name: "bbrk",
        value: "\u{23B5}",
        legacy: false,
    },
    NamedRef {
        name: "bbrktbrk",
        value: "\u{23B6}",
        legacy: false,
    },
    NamedRef {
        name: "bcong",
        value: "\u{224C}",
        legacy: false,
    },
    NamedRef {
        name: "bcy",
        value: "\u{431}",
        legacy: false,
    },
    NamedRef {
        name: "bdquo",
        value: "\u{201E}",
        legacy: false,
    },
    NamedRef {
        name: "becaus",
        value: "\u{2235}",
        legacy: false,
    },
    NamedRef {
        name: "because",
        value: "\u{2235}",
        legacy: false,
    },
    NamedRef {
        name: "bemptyv",
        value: "\u{29B0}",
        legacy: false,
    },
    NamedRef {
        name: "bepsi",
        value: "\u{3F6}",
        legacy: false,
    },
    NamedRef {
        name: "bernou",
        value: "\u{212C}",
        legacy: false,
    },
    NamedRef {
        name: "beta",
        value: "\u{3B2}",
        legacy: false,
    },
    NamedRef {
        name: "beth",
        value: "\u{2136}",
        legacy: false,
    },
    NamedRef {
        name: "between",
        value: "\u{226C}",
        legacy: false,
    },
    NamedRef {
        name: "bfr",
        value: "\u{1D51F}",
        legacy: false,
    },
    NamedRef {
        name: "bigcap",
        value: "\u{22C2}",
        legacy: false,
    },
    NamedRef {
        name: "bigcirc",
        value: "\u{25EF}",
        legacy: false,
    },
    NamedRef {
        name: "bigcup",
        value: "\u{22C3}",
        legacy: false,
    },
    NamedRef {
        name: "bigodot",
        value: "\u{2A00}",
        legacy: false,
    },
    NamedRef {
        name: "bigoplus",
        value: "\u{2A01}",
        legacy: false,
    },
    NamedRef {
        name: "bigotimes",
        value: "\u{2A02}",
        legacy: false,
    },
    NamedRef {
        name: "bigsqcup",
        value: "\u{2A06}",
        legacy: false,
    },
    NamedRef {
        name: "bigstar",
        value: "\u{2605}",
        legacy: false,
    },
    NamedRef {
        name: "bigtriangledown",
        value: "\u{25BD}",
        legacy: false,
    },
    NamedRef {
        name: "bigtriangleup",
        value: "\u{25B3}",
        legacy: false,
    },
    NamedRef {
        name: "biguplus",
        value: "\u{2A04}",
        legacy: false,
    },
    NamedRef {
        name: "bigvee",
        value: "\u{22C1}",
        legacy: false,
    },
    NamedRef {
        name: "bigwedge",
        value: "\u{22C0}",
        legacy: false,
    },
    NamedRef {
        name: "bkarow",
        value: "\u{290D}",
        legacy: false,
    },
    NamedRef {
        name: "blacklozenge",
        value: "\u{29EB}",
        legacy: false,
    },
    NamedRef {
        name: "blacksquare",
        value: "\u{25AA}",
        legacy: false,
    },
    NamedRef {
        name: "blacktriangle",
        value: "\u{25B4}",
        legacy: false,
    },
    NamedRef {
        name: "blacktriangledown",
        value: "\u{25BE}",
        legacy: false,
    },
    NamedRef {
        name: "blacktriangleleft",
        value: "\u{25C2}",
        legacy: false,
    },
    NamedRef {
        name: "blacktriangleright",
        value: "\u{25B8}",
        legacy: false,
    },
    NamedRef {
        name: "blank",
        value: "\u{2423}",
        legacy: false,
    },
    NamedRef {
        name: "blk12",
        value: "\u{2592}",
        legacy: false,
    },
    NamedRef {
        name: "blk14",
        value: "\u{2591}",
        legacy: false,
    },
    NamedRef {
        name: "blk34",
        value: "\u{2593}",
        legacy: false,
    },
    NamedRef {
        name: "block",
        value: "\u{2588}",
        legacy: false,
    },
    NamedRef {
        name: "bne",
        value: "\u{3D}\u{20E5}",
        legacy: false,
    },
    NamedRef {
        name: "bnequiv",
        value: "\u{2261}\u{20E5}",
        legacy: false,
    },
    NamedRef {
        name: "bnot",
        value: "\u{2310}",
        legacy: false,
    },
    NamedRef {
        name: "bopf",
        value: "\u{1D553}",
        legacy: false,
    },
    NamedRef {
        name: "bot",
        value: "\u{22A5}",
        legacy: false,
    },
    NamedRef {
        name: "bottom",
        value: "\u{22A5}",
        legacy: false,
    },
    NamedRef {
        name: "bowtie",
        value: "\u{22C8}",
        legacy: false,
    },
    NamedRef {
        name: "boxDL",
        value: "\u{2557}",
        legacy: false,
    },
    NamedRef {
        name: "boxDR",
        value: "\u{2554}",
        legacy: false,
    },
    NamedRef {
        name: "boxDl",
        value: "\u{2556}",
        legacy: false,
    },
    NamedRef {
        name: "boxDr",
        value: "\u{2553}",
        legacy: false,
    },
    NamedRef {
        name: "boxH",
        value: "\u{2550}",
        legacy: false,
    },
    NamedRef {
        name: "boxHD",
        value: "\u{2566}",
        legacy: false,
    },
    NamedRef {
        name: "boxHU",
        value: "\u{2569}",
        legacy: false,
    },
    NamedRef {
        name: "boxHd",
        value: "\u{2564}",
        legacy: false,
    },
    NamedRef {
        name: "boxHu",
        value: "\u{2567}",
        legacy: false,
    },
    NamedRef {
        name: "boxUL",
        value: "\u{255D}",
        legacy: false,
    },
    NamedRef {
        name: "boxUR",
        value: "\u{255A}",
        legacy: false,
    },
    NamedRef {
        name: "boxUl",
        value: "\u{255C}",
        legacy: false,
    },
    NamedRef {
        name: "boxUr",
        value: "\u{2559}",
        legacy: false,
    },
    NamedRef {
        name: "boxV",
        value: "\u{2551}",
        legacy: false,
    },
    NamedRef {
        name: "boxVH",
        value: "\u{256C}",
        legacy: false,
    },
    NamedRef {
        name: "boxVL",
        value: "\u{2563}",
        legacy: false,
    },
    NamedRef {
        name: "boxVR",
        value: "\u{2560}",
        legacy: false,
    },
    NamedRef {
        name: "boxVh",
        value: "\u{256B}",
        legacy: false,
    },
    NamedRef {
        name: "boxVl",
        value: "\u{2562}",
        legacy: false,
    },
    NamedRef {
        name: "boxVr",
        value: "\u{255F}",
        legacy: false,
    },
    NamedRef {
        name: "boxbox",
        value: "\u{29C9}",
        legacy: false,
    },
    NamedRef {
        name: "boxdL",
        value: "\u{2555}",
        legacy: false,
    },
    NamedRef {
        name: "boxdR",
        value: "\u{2552}",
        legacy: false,
    },
    NamedRef {
        name: "boxdl",
        value: "\u{2510}",
        legacy: false,
    },
    NamedRef {
        name: "boxdr",
        value: "\u{250C}",
        legacy: false,
    },
    NamedRef {
        name: "boxh",
        value: "\u{2500}",
        legacy: false,
    },
    NamedRef {
        name: "boxhD",
        value: "\u{2565}",
        legacy: false,
    },
    NamedRef {
        name: "boxhU",
        value: "\u{2568}",
        legacy: false,
    },
    NamedRef {
        name: "boxhd",
        value: "\u{252C}",
        legacy: false,
    },
    NamedRef {
        name: "boxhu",
        value: "\u{2534}",
        legacy: false,
    },
    NamedRef {
        name: "boxminus",
        value: "\u{229F}",
        legacy: false,
    },
    NamedRef {
        name: "boxplus",
        value: "\u{229E}",
        legacy: false,
    },
    NamedRef {
        name: "boxtimes",
        value: "\u{22A0}",
        legacy: false,
    },
    NamedRef {
        name: "boxuL",
        value: "\u{255B}",
        legacy: false,
    },
    NamedRef {
        name: "boxuR",
        value: "\u{2558}",
        legacy: false,
    },
    NamedRef {
        name: "boxul",
        value: "\u{2518}",
        legacy: false,
    },
    NamedRef {
        name: "boxur",
        value: "\u{2514}",
        legacy: false,
    },
    NamedRef {
        name: "boxv",
        value: "\u{2502}",
        legacy: false,
    },
    NamedRef {
        name: "boxvH",
        value: "\u{256A}",
        legacy: false,
    },
    NamedRef {
        name: "boxvL",
        value: "\u{2561}",
        legacy: false,
    },
    NamedRef {
        name: "boxvR",
        value: "\u{255E}",
        legacy: false,
    },
    NamedRef {
        name: "boxvh",
        value: "\u{253C}",
        legacy: false,
    },
    NamedRef {
        name: "boxvl",
        value: "\u{2524}",
        legacy: false,
    },
    NamedRef {
        name: "boxvr",
        value: "\u{251C}",
        legacy: false,
    },
    NamedRef {
        name: "bprime",
        value: "\u{2035}",
        legacy: false,
    },
    NamedRef {
        name: "breve",
        value: "\u{2D8}",
        legacy: false,
    },
    NamedRef {
        name: "brvbar",
        value: "\u{A6}",
        legacy: true,
    },
    NamedRef {
        name: "bscr",
        value: "\u{1D4B7}",
        legacy: false,
    },
    NamedRef {
        name: "bsemi",
        value: "\u{204F}",
        legacy: false,
    },
    NamedRef {
        name: "bsim",
        value: "\u{223D}",
        legacy: false,
    },
    NamedRef {
        name: "bsime",
        value: "\u{22CD}",
        legacy: false,
    },
    NamedRef {
        name: "bsol",
        value: "\u{5C}",
        legacy: false,
    },
    NamedRef {
        name: "bsolb",
        value: "\u{29C5}",
        legacy: false,
    },
    NamedRef {
        name: "bsolhsub",
        value: "\u{27C8}",
        legacy: false,
    },
    NamedRef {
        name: "bull",
        value: "\u{2022}",
        legacy: false,
    },
    NamedRef {
        name: "bullet",
        value: "\u{2022}",
        legacy: false,
    },
    NamedRef {
        name: "bump",
        value: "\u{224E}",
        legacy: false,
    },
    NamedRef {
        name: "bumpE",
        value: "\u{2AAE}",
        legacy: false,
    },
    NamedRef {
        name: "bumpe",
        value: "\u{224F}",
        legacy: false,
    },
    NamedRef {
        name: "bumpeq",
        value: "\u{224F}",
        legacy: false,
    },
    NamedRef {
        name: "cacute",
        value: "\u{107}",
        legacy: false,
    },
    NamedRef {
        name: "cap",
        value: "\u{2229}",
        legacy: false,
    },
    NamedRef {
        name: "capand",
        value: "\u{2A44}",
        legacy: false,
    },
    NamedRef {
        name: "capbrcup",
        value: "\u{2A49}",
        legacy: false,
    },
    NamedRef {
        name: "capcap",
        value: "\u{2A4B}",
        legacy: false,
    },
    NamedRef {
        name: "capcup",
        value: "\u{2A47}",
        legacy: false,
    },
    NamedRef {
        name: "capdot",
        value: "\u{2A40}",
        legacy: false,
    },
    NamedRef {
        name: "caps",
        value: "\u{2229}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "caret",
        value: "\u{2041}",
        legacy: false,
    },
    NamedRef {
        name: "caron",
        value: "\u{2C7}",
        legacy: false,
    },
    NamedRef {
        name: "ccaps",
        value: "\u{2A4D}",
        legacy: false,
    },
    NamedRef {
        name: "ccaron",
        value: "\u{10D}",
        legacy: false,
    },
    NamedRef {
        name: "ccedil",
        value: "\u{E7}",
        legacy: true,
    },
    NamedRef {
        name: "ccirc",
        value: "\u{109}",
        legacy: false,
    },
    NamedRef {
        name: "ccups",
        value: "\u{2A4C}",
        legacy: false,
    },
    NamedRef {
        name: "ccupssm",
        value: "\u{2A50}",
        legacy: false,
    },
    NamedRef {
        name: "cdot",
        value: "\u{10B}",
        legacy: false,
    },
    NamedRef {
        name: "cedil",
        value: "\u{B8}",
        legacy: true,
    },
    NamedRef {
        name: "cemptyv",
        value: "\u{29B2}",
        legacy: false,
    },
    NamedRef {
        name: "cent",
        value: "\u{A2}",
        legacy: true,
    },
    NamedRef {
        name: "centerdot",
        value: "\u{B7}",
        legacy: false,
    },
    NamedRef {
        name: "cfr",
        value: "\u{1D520}",
        legacy: false,
    },
    NamedRef {
        name: "chcy",
        value: "\u{447}",
        legacy: false,
    },
    NamedRef {
        name: "check",
        value: "\u{2713}",
        legacy: false,
    },
    NamedRef {
        name: "checkmark",
        value: "\u{2713}",
        legacy: false,
    },
    NamedRef {
        name: "chi",
        value: "\u{3C7}",
        legacy: false,
    },
    NamedRef {
        name: "cir",
        value: "\u{25CB}",
        legacy: false,
    },
    NamedRef {
        name: "cirE",
        value: "\u{29C3}",
        legacy: false,
    },
    NamedRef {
        name: "circ",
        value: "\u{2C6}",
        legacy: false,
    },
    NamedRef {
        name: "circeq",
        value: "\u{2257}",
        legacy: false,
    },
    NamedRef {
        name: "circlearrowleft",
        value: "\u{21BA}",
        legacy: false,
    },
    NamedRef {
        name: "circlearrowright",
        value: "\u{21BB}",
        legacy: false,
    },
    NamedRef {
        name: "circledR",
        value: "\u{AE}",
        legacy: false,
    },
    NamedRef {
        name: "circledS",
        value: "\u{24C8}",
        legacy: false,
    },
    NamedRef {
        name: "circledast",
        value: "\u{229B}",
        legacy: false,
    },
    NamedRef {
        name: "circledcirc",
        value: "\u{229A}",
        legacy: false,
    },
    NamedRef {
        name: "circleddash",
        value: "\u{229D}",
        legacy: false,
    },
    NamedRef {
        name: "cire",
        value: "\u{2257}",
        legacy: false,
    },
    NamedRef {
        name: "cirfnint",
        value: "\u{2A10}",
        legacy: false,
    },
    NamedRef {
        name: "cirmid",
        value: "\u{2AEF}",
        legacy: false,
    },
    NamedRef {
        name: "cirscir",
        value: "\u{29C2}",
        legacy: false,
    },
    NamedRef {
        name: "clubs",
        value: "\u{2663}",
        legacy: false,
    },
    NamedRef {
        name: "clubsuit",
        value: "\u{2663}",
        legacy: false,
    },
    NamedRef {
        name: "colon",
        value: "\u{3A}",
        legacy: false,
    },
    NamedRef {
        name: "colone",
        value: "\u{2254}",
        legacy: false,
    },
    NamedRef {
        name: "coloneq",
        value: "\u{2254}",
        legacy: false,
    },
    NamedRef {
        name: "comma",
        value: "\u{2C}",
        legacy: false,
    },
    NamedRef {
        name: "commat",
        value: "\u{40}",
        legacy: false,
    },
    NamedRef {
        name: "comp",
        value: "\u{2201}",
        legacy: false,
    },
    NamedRef {
        name: "compfn",
        value: "\u{2218}",
        legacy: false,
    },
    NamedRef {
        name: "complement",
        value: "\u{2201}",
        legacy: false,
    },
    NamedRef {
        name: "complexes",
        value: "\u{2102}",
        legacy: false,
    },
    NamedRef {
        name: "cong",
        value: "\u{2245}",
        legacy: false,
    },
    NamedRef {
        name: "congdot",
        value: "\u{2A6D}",
        legacy: false,
    },
    NamedRef {
        name: "conint",
        value: "\u{222E}",
        legacy: false,
    },
    NamedRef {
        name: "copf",
        value: "\u{1D554}",
        legacy: false,
    },
    NamedRef {
        name: "coprod",
        value: "\u{2210}",
        legacy: false,
    },
    NamedRef {
        name: "copy",
        value: "\u{A9}",
        legacy: true,
    },
    NamedRef {
        name: "copysr",
        value: "\u{2117}",
        legacy: false,
    },
    NamedRef {
        name: "crarr",
        value: "\u{21B5}",
        legacy: false,
    },
    NamedRef {
        name: "cross",
        value: "\u{2717}",
        legacy: false,
    },
    NamedRef {
        name: "cscr",
        value: "\u{1D4B8}",
        legacy: false,
    },
    NamedRef {
        name: "csub",
        value: "\u{2ACF}",
        legacy: false,
    },
    NamedRef {
        name: "csube",
        value: "\u{2AD1}",
        legacy: false,
    },
    NamedRef {
        name: "csup",
        value: "\u{2AD0}",
        legacy: false,
    },
    NamedRef {
        name: "csupe",
        value: "\u{2AD2}",
        legacy: false,
    },
    NamedRef {
        name: "ctdot",
        value: "\u{22EF}",
        legacy: false,
    },
    NamedRef {
        name: "cudarrl",
        value: "\u{2938}",
        legacy: false,
    },
    NamedRef {
        name: "cudarrr",
        value: "\u{2935}",
        legacy: false,
    },
    NamedRef {
        name: "cuepr",
        value: "\u{22DE}",
        legacy: false,
    },
    NamedRef {
        name: "cuesc",
        value: "\u{22DF}",
        legacy: false,
    },
    NamedRef {
        name: "cularr",
        value: "\u{21B6}",
        legacy: false,
    },
    NamedRef {
        name: "cularrp",
        value: "\u{293D}",
        legacy: false,
    },
    NamedRef {
        name: "cup",
        value: "\u{222A}",
        legacy: false,
    },
    NamedRef {
        name: "cupbrcap",
        value: "\u{2A48}",
        legacy: false,
    },
    NamedRef {
        name: "cupcap",
        value: "\u{2A46}",
        legacy: false,
    },
    NamedRef {
        name: "cupcup",
        value: "\u{2A4A}",
        legacy: false,
    },
    NamedRef {
        name: "cupdot",
        value: "\u{228D}",
        legacy: false,
    },
    NamedRef {
        name: "cupor",
        value: "\u{2A45}",
        legacy: false,
    },
    NamedRef {
        name: "cups",
        value: "\u{222A}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "curarr",
        value: "\u{21B7}",
        legacy: false,
    },
    NamedRef {
        name: "curarrm",
        value: "\u{293C}",
        legacy: false,
    },
    NamedRef {
        name: "curlyeqprec",
        value: "\u{22DE}",
        legacy: false,
    },
    NamedRef {
        name: "curlyeqsucc",
        value: "\u{22DF}",
        legacy: false,
    },
    NamedRef {
        name: "curlyvee",
        value: "\u{22CE}",
        legacy: false,
    },
    NamedRef {
        name: "curlywedge",
        value: "\u{22CF}",
        legacy: false,
    },
    NamedRef {
        name: "curren",
        value: "\u{A4}",
        legacy: true,
    },
    NamedRef {
        name: "curvearrowleft",
        value: "\u{21B6}",
        legacy: false,
    },
    NamedRef {
        name: "curvearrowright",
        value: "\u{21B7}",
        legacy: false,
    },
    NamedRef {
        name: "cuvee",
        value: "\u{22CE}",
        legacy: false,
    },
    NamedRef {
        name: "cuwed",
        value: "\u{22CF}",
        legacy: false,
    },
    NamedRef {
        name: "cwconint",
        value: "\u{2232}",
        legacy: false,
    },
    NamedRef {
        name: "cwint",
        value: "\u{2231}",
        legacy: false,
    },
    NamedRef {
        name: "cylcty",
        value: "\u{232D}",
        legacy: false,
    },
    NamedRef {
        name: "dArr",
        value: "\u{21D3}",
        legacy: false,
    },
    NamedRef {
        name: "dHar",
        value: "\u{2965}",
        legacy: false,
    },
    NamedRef {
        name: "dagger",
        value: "\u{2020}",
        legacy: false,
    },
    NamedRef {
        name: "daleth",
        value: "\u{2138}",
        legacy: false,
    },
    NamedRef {
        name: "darr",
        value: "\u{2193}",
        legacy: false,
    },
    NamedRef {
        name: "dash",
        value: "\u{2010}",
        legacy: false,
    },
    NamedRef {
        name: "dashv",
        value: "\u{22A3}",
        legacy: false,
    },
    NamedRef {
        name: "dbkarow",
        value: "\u{290F}",
        legacy: false,
    },
    NamedRef {
        name: "dblac",
        value: "\u{2DD}",
        legacy: false,
    },
    NamedRef {
        name: "dcaron",
        value: "\u{10F}",
        legacy: false,
    },
    NamedRef {
        name: "dcy",
        value: "\u{434}",
        legacy: false,
    },
    NamedRef {
        name: "dd",
        value: "\u{2146}",
        legacy: false,
    },
    NamedRef {
        name: "ddagger",
        value: "\u{2021}",
        legacy: false,
    },
    NamedRef {
        name: "ddarr",
        value: "\u{21CA}",
        legacy: false,
    },
    NamedRef {
        name: "ddotseq",
        value: "\u{2A77}",
        legacy: false,
    },
    NamedRef {
        name: "deg",
        value: "\u{B0}",
        legacy: true,
    },
    NamedRef {
        name: "delta",
        value: "\u{3B4}",
        legacy: false,
    },
    NamedRef {
        name: "demptyv",
        value: "\u{29B1}",
        legacy: false,
    },
    NamedRef {
        name: "dfisht",
        value: "\u{297F}",
        legacy: false,
    },
    NamedRef {
        name: "dfr",
        value: "\u{1D521}",
        legacy: false,
    },
    NamedRef {
        name: "dharl",
        value: "\u{21C3}",
        legacy: false,
    },
    NamedRef {
        name: "dharr",
        value: "\u{21C2}",
        legacy: false,
    },
    NamedRef {
        name: "diam",
        value: "\u{22C4}",
        legacy: false,
    },
    NamedRef {
        name: "diamond",
        value: "\u{22C4}",
        legacy: false,
    },
    NamedRef {
        name: "diamondsuit",
        value: "\u{2666}",
        legacy: false,
    },
    NamedRef {
        name: "diams",
        value: "\u{2666}",
        legacy: false,
    },
    NamedRef {
        name: "die",
        value: "\u{A8}",
        legacy: false,
    },
    NamedRef {
        name: "digamma",
        value: "\u{3DD}",
        legacy: false,
    },
    NamedRef {
        name: "disin",
        value: "\u{22F2}",
        legacy: false,
    },
    NamedRef {
        name: "div",
        value: "\u{F7}",
        legacy: false,
    },
    NamedRef {
        name: "divide",
        value: "\u{F7}",
        legacy: true,
    },
    NamedRef {
        name: "divideontimes",
        value: "\u{22C7}",
        legacy: false,
    },
    NamedRef {
        name: "divonx",
        value: "\u{22C7}",
        legacy: false,
    },
    NamedRef {
        name: "djcy",
        value: "\u{452}",
        legacy: false,
    },
    NamedRef {
        name: "dlcorn",
        value: "\u{231E}",
        legacy: false,
    },
    NamedRef {
        name: "dlcrop",
        value: "\u{230D}",
        legacy: false,
    },
    NamedRef {
        name: "dollar",
        value: "\u{24}",
        legacy: false,
    },
    NamedRef {
        name: "dopf",
        value: "\u{1D555}",
        legacy: false,
    },
    NamedRef {
        name: "dot",
        value: "\u{2D9}",
        legacy: false,
    },
    NamedRef {
        name: "doteq",
        value: "\u{2250}",
        legacy: false,
    },
    NamedRef {
        name: "doteqdot",
        value: "\u{2251}",
        legacy: false,
    },
    NamedRef {
        name: "dotminus",
        value: "\u{2238}",
        legacy: false,
    },
    NamedRef {
        name: "dotplus",
        value: "\u{2214}",
        legacy: false,
    },
    NamedRef {
        name: "dotsquare",
        value: "\u{22A1}",
        legacy: false,
    },
    NamedRef {
        name: "doublebarwedge",
        value: "\u{2306}",
        legacy: false,
    },
    NamedRef {
        name: "downarrow",
        value: "\u{2193}",
        legacy: false,
    },
    NamedRef {
        name: "downdownarrows",
        value: "\u{21CA}",
        legacy: false,
    },
    NamedRef {
        name: "downharpoonleft",
        value: "\u{21C3}",
        legacy: false,
    },
    NamedRef {
        name: "downharpoonright",
        value: "\u{21C2}",
        legacy: false,
    },
    NamedRef {
        name: "drbkarow",
        value: "\u{2910}",
        legacy: false,
    },
    NamedRef {
        name: "drcorn",
        value: "\u{231F}",
        legacy: false,
    },
    NamedRef {
        name: "drcrop",
        value: "\u{230C}",
        legacy: false,
    },
    NamedRef {
        name: "dscr",
        value: "\u{1D4B9}",
        legacy: false,
    },
    NamedRef {
        name: "dscy",
        value: "\u{455}",
        legacy: false,
    },
    NamedRef {
        name: "dsol",
        value: "\u{29F6}",
        legacy: false,
    },
    NamedRef {
        name: "dstrok",
        value: "\u{111}",
        legacy: false,
    },
    NamedRef {
        name: "dtdot",
        value: "\u{22F1}",
        legacy: false,
    },
    NamedRef {
        name: "dtri",
        value: "\u{25BF}",
        legacy: false,
    },
    NamedRef {
        name: "dtrif",
        value: "\u{25BE}",
        legacy: false,
    },
    NamedRef {
        name: "duarr",
        value: "\u{21F5}",
        legacy: false,
    },
    NamedRef {
        name: "duhar",
        value: "\u{296F}",
        legacy: false,
    },
    NamedRef {
        name: "dwangle",
        value: "\u{29A6}",
        legacy: false,
    },
    NamedRef {
        name: "dzcy",
        value: "\u{45F}",
        legacy: false,
    },
    NamedRef {
        name: "dzigrarr",
        value: "\u{27FF}",
        legacy: false,
    },
    NamedRef {
        name: "eDDot",
        value: "\u{2A77}",
        legacy: false,
    },
    NamedRef {
        name: "eDot",
        value: "\u{2251}",
        legacy: false,
    },
    NamedRef {
        name: "eacute",
        value: "\u{E9}",
        legacy: true,
    },
    NamedRef {
        name: "easter",
        value: "\u{2A6E}",
        legacy: false,
    },
    NamedRef {
        name: "ecaron",
        value: "\u{11B}",
        legacy: false,
    },
    NamedRef {
        name: "ecir",
        value: "\u{2256}",
        legacy: false,
    },
    NamedRef {
        name: "ecirc",
        value: "\u{EA}",
        legacy: true,
    },
    NamedRef {
        name: "ecolon",
        value: "\u{2255}",
        legacy: false,
    },
    NamedRef {
        name: "ecy",
        value: "\u{44D}",
        legacy: false,
    },
    NamedRef {
        name: "edot",
        value: "\u{117}",
        legacy: false,
    },
    NamedRef {
        name: "ee",
        value: "\u{2147}",
        legacy: false,
    },
    NamedRef {
        name: "efDot",
        value: "\u{2252}",
        legacy: false,
    },
    NamedRef {
        name: "efr",
        value: "\u{1D522}",
        legacy: false,
    },
    NamedRef {
        name: "eg",
        value: "\u{2A9A}",
        legacy: false,
    },
    NamedRef {
        name: "egrave",
        value: "\u{E8}",
        legacy: true,
    },
    NamedRef {
        name: "egs",
        value: "\u{2A96}",
        legacy: false,
    },
    NamedRef {
        name: "egsdot",
        value: "\u{2A98}",
        legacy: false,
    },
    NamedRef {
        name: "el",
        value: "\u{2A99}",
        legacy: false,
    },
    NamedRef {
        name: "elinters",
        value: "\u{23E7}",
        legacy: false,
    },
    NamedRef {
        name: "ell",
        value: "\u{2113}",
        legacy: false,
    },
    NamedRef {
        name: "els",
        value: "\u{2A95}",
        legacy: false,
    },
    NamedRef {
        name: "elsdot",
        value: "\u{2A97}",
        legacy: false,
    },
    NamedRef {
        name: "emacr",
        value: "\u{113}",
        legacy: false,
    },
    NamedRef {
        name: "empty",
        value: "\u{2205}",
        legacy: false,
    },
    NamedRef {
        name: "emptyset",
        value: "\u{2205}",
        legacy: false,
    },
    NamedRef {
        name: "emptyv",
        value: "\u{2205}",
        legacy: false,
    },
    NamedRef {
        name: "emsp",
        value: "\u{2003}",
        legacy: false,
    },
    NamedRef {
        name: "emsp13",
        value: "\u{2004}",
        legacy: false,
    },
    NamedRef {
        name: "emsp14",
        value: "\u{2005}",
        legacy: false,
    },
    NamedRef {
        name: "eng",
        value: "\u{14B}",
        legacy: false,
    },
    NamedRef {
        name: "ensp",
        value: "\u{2002}",
        legacy: false,
    },
    NamedRef {
        name: "eogon",
        value: "\u{119}",
        legacy: false,
    },
    NamedRef {
        name: "eopf",
        value: "\u{1D556}",
        legacy: false,
    },
    NamedRef {
        name: "epar",
        value: "\u{22D5}",
        legacy: false,
    },
    NamedRef {
        name: "eparsl",
        value: "\u{29E3}",
        legacy: false,
    },
    NamedRef {
        name: "eplus",
        value: "\u{2A71}",
        legacy: false,
    },
    NamedRef {
        name: "epsi",
        value: "\u{3B5}",
        legacy: false,
    },
    NamedRef {
        name: "epsilon",
        value: "\u{3B5}",
        legacy: false,
    },
    NamedRef {
        name: "epsiv",
        value: "\u{3F5}",
        legacy: false,
    },
    NamedRef {
        name: "eqcirc",
        value: "\u{2256}",
        legacy: false,
    },
    NamedRef {
        name: "eqcolon",
        value: "\u{2255}",
        legacy: false,
    },
    NamedRef {
        name: "eqsim",
        value: "\u{2242}",
        legacy: false,
    },
    NamedRef {
        name: "eqslantgtr",
        value: "\u{2A96}",
        legacy: false,
    },
    NamedRef {
        name: "eqslantless",
        value: "\u{2A95}",
        legacy: false,
    },
    NamedRef {
        name: "equals",
        value: "\u{3D}",
        legacy: false,
    },
    NamedRef {
        name: "equest",
        value: "\u{225F}",
        legacy: false,
    },
    NamedRef {
        name: "equiv",
        value: "\u{2261}",
        legacy: false,
    },
    NamedRef {
        name: "equivDD",
        value: "\u{2A78}",
        legacy: false,
    },
    NamedRef {
        name: "eqvparsl",
        value: "\u{29E5}",
        legacy: false,
    },
    NamedRef {
        name: "erDot",
        value: "\u{2253}",
        legacy: false,
    },
    NamedRef {
        name: "erarr",
        value: "\u{2971}",
        legacy: false,
    },
    NamedRef {
        name: "escr",
        value: "\u{212F}",
        legacy: false,
    },
    NamedRef {
        name: "esdot",
        value: "\u{2250}",
        legacy: false,
    },
    NamedRef {
        name: "esim",
        value: "\u{2242}",
        legacy: false,
    },
    NamedRef {
        name: "eta",
        value: "\u{3B7}",
        legacy: false,
    },
    NamedRef {
        name: "eth",
        value: "\u{F0}",
        legacy: true,
    },
    NamedRef {
        name: "euml",
        value: "\u{EB}",
        legacy: true,
    },
    NamedRef {
        name: "euro",
        value: "\u{20AC}",
        legacy: false,
    },
    NamedRef {
        name: "excl",
        value: "\u{21}",
        legacy: false,
    },
    NamedRef {
        name: "exist",
        value: "\u{2203}",
        legacy: false,
    },
    NamedRef {
        name: "expectation",
        value: "\u{2130}",
        legacy: false,
    },
    NamedRef {
        name: "exponentiale",
        value: "\u{2147}",
        legacy: false,
    },
    NamedRef {
        name: "fallingdotseq",
        value: "\u{2252}",
        legacy: false,
    },
    NamedRef {
        name: "fcy",
        value: "\u{444}",
        legacy: false,
    },
    NamedRef {
        name: "female",
        value: "\u{2640}",
        legacy: false,
    },
    NamedRef {
        name: "ffilig",
        value: "\u{FB03}",
        legacy: false,
    },
    NamedRef {
        name: "fflig",
        value: "\u{FB00}",
        legacy: false,
    },
    NamedRef {
        name: "ffllig",
        value: "\u{FB04}",
        legacy: false,
    },
    NamedRef {
        name: "ffr",
        value: "\u{1D523}",
        legacy: false,
    },
    NamedRef {
        name: "filig",
        value: "\u{FB01}",
        legacy: false,
    },
    NamedRef {
        name: "fjlig",
        value: "\u{66}\u{6A}",
        legacy: false,
    },
    NamedRef {
        name: "flat",
        value: "\u{266D}",
        legacy: false,
    },
    NamedRef {
        name: "fllig",
        value: "\u{FB02}",
        legacy: false,
    },
    NamedRef {
        name: "fltns",
        value: "\u{25B1}",
        legacy: false,
    },
    NamedRef {
        name: "fnof",
        value: "\u{192}",
        legacy: false,
    },
    NamedRef {
        name: "fopf",
        value: "\u{1D557}",
        legacy: false,
    },
    NamedRef {
        name: "forall",
        value: "\u{2200}",
        legacy: false,
    },
    NamedRef {
        name: "fork",
        value: "\u{22D4}",
        legacy: false,
    },
    NamedRef {
        name: "forkv",
        value: "\u{2AD9}",
        legacy: false,
    },
    NamedRef {
        name: "fpartint",
        value: "\u{2A0D}",
        legacy: false,
    },
    NamedRef {
        name: "frac12",
        value: "\u{BD}",
        legacy: true,
    },
    NamedRef {
        name: "frac13",
        value: "\u{2153}",
        legacy: false,
    },
    NamedRef {
        name: "frac14",
        value: "\u{BC}",
        legacy: true,
    },
    NamedRef {
        name: "frac15",
        value: "\u{2155}",
        legacy: false,
    },
    NamedRef {
        name: "frac16",
        value: "\u{2159}",
        legacy: false,
    },
    NamedRef {
        name: "frac18",
        value: "\u{215B}",
        legacy: false,
    },
    NamedRef {
        name: "frac23",
        value: "\u{2154}",
        legacy: false,
    },
    NamedRef {
        name: "frac25",
        value: "\u{2156}",
        legacy: false,
    },
    NamedRef {
        name: "frac34",
        value: "\u{BE}",
        legacy: true,
    },
    NamedRef {
        name: "frac35",
        value: "\u{2157}",
        legacy: false,
    },
    NamedRef {
        name: "frac38",
        value: "\u{215C}",
        legacy: false,
    },
    NamedRef {
        name: "frac45",
        value: "\u{2158}",
        legacy: false,
    },
    NamedRef {
        name: "frac56",
        value: "\u{215A}",
        legacy: false,
    },
    NamedRef {
        name: "frac58",
        value: "\u{215D}",
        legacy: false,
    },
    NamedRef {
        name: "frac78",
        value: "\u{215E}",
        legacy: false,
    },
    NamedRef {
        name: "frasl",
        value: "\u{2044}",
        legacy: false,
    },
    NamedRef {
        name: "frown",
        value: "\u{2322}",
        legacy: false,
    },
    NamedRef {
        name: "fscr",
        value: "\u{1D4BB}",
        legacy: false,
    },
    NamedRef {
        name: "gE",
        value: "\u{2267}",
        legacy: false,
    },
    NamedRef {
        name: "gEl",
        value: "\u{2A8C}",
        legacy: false,
    },
    NamedRef {
        name: "gacute",
        value: "\u{1F5}",
        legacy: false,
    },
    NamedRef {
        name: "gamma",
        value: "\u{3B3}",
        legacy: false,
    },
    NamedRef {
        name: "gammad",
        value: "\u{3DD}",
        legacy: false,
    },
    NamedRef {
        name: "gap",
        value: "\u{2A86}",
        legacy: false,
    },
    NamedRef {
        name: "gbreve",
        value: "\u{11F}",
        legacy: false,
    },
    NamedRef {
        name: "gcirc",
        value: "\u{11D}",
        legacy: false,
    },
    NamedRef {
        name: "gcy",
        value: "\u{433}",
        legacy: false,
    },
    NamedRef {
        name: "gdot",
        value: "\u{121}",
        legacy: false,
    },
    NamedRef {
        name: "ge",
        value: "\u{2265}",
        legacy: false,
    },
    NamedRef {
        name: "gel",
        value: "\u{22DB}",
        legacy: false,
    },
    NamedRef {
        name: "geq",
        value: "\u{2265}",
        legacy: false,
    },
    NamedRef {
        name: "geqq",
        value: "\u{2267}",
        legacy: false,
    },
    NamedRef {
        name: "geqslant",
        value: "\u{2A7E}",
        legacy: false,
    },
    NamedRef {
        name: "ges",
        value: "\u{2A7E}",
        legacy: false,
    },
    NamedRef {
        name: "gescc",
        value: "\u{2AA9}",
        legacy: false,
    },
    NamedRef {
        name: "gesdot",
        value: "\u{2A80}",
        legacy: false,
    },
    NamedRef {
        name: "gesdoto",
        value: "\u{2A82}",
        legacy: false,
    },
    NamedRef {
        name: "gesdotol",
        value: "\u{2A84}",
        legacy: false,
    },
    NamedRef {
        name: "gesl",
        value: "\u{22DB}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "gesles",
        value: "\u{2A94}",
        legacy: false,
    },
    NamedRef {
        name: "gfr",
        value: "\u{1D524}",
        legacy: false,
    },
    NamedRef {
        name: "gg",
        value: "\u{226B}",
        legacy: false,
    },
    NamedRef {
        name: "ggg",
        value: "\u{22D9}",
        legacy: false,
    },
    NamedRef {
        name: "gimel",
        value: "\u{2137}",
        legacy: false,
    },
    NamedRef {
        name: "gjcy",
        value: "\u{453}",
        legacy: false,
    },
    NamedRef {
        name: "gl",
        value: "\u{2277}",
        legacy: false,
    },
    NamedRef {
        name: "glE",
        value: "\u{2A92}",
        legacy: false,
    },
    NamedRef {
        name: "gla",
        value: "\u{2AA5}",
        legacy: false,
    },
    NamedRef {
        name: "glj",
        value: "\u{2AA4}",
        legacy: false,
    },
    NamedRef {
        name: "gnE",
        value: "\u{2269}",
        legacy: false,
    },
    NamedRef {
        name: "gnap",
        value: "\u{2A8A}",
        legacy: false,
    },
    NamedRef {
        name: "gnapprox",
        value: "\u{2A8A}",
        legacy: false,
    },
    NamedRef {
        name: "gne",
        value: "\u{2A88}",
        legacy: false,
    },
    NamedRef {
        name: "gneq",
        value: "\u{2A88}",
        legacy: false,
    },
    NamedRef {
        name: "gneqq",
        value: "\u{2269}",
        legacy: false,
    },
    NamedRef {
        name: "gnsim",
        value: "\u{22E7}",
        legacy: false,
    },
    NamedRef {
        name: "gopf",
        value: "\u{1D558}",
        legacy: false,
    },
    NamedRef {
        name: "grave",
        value: "\u{60}",
        legacy: false,
    },
    NamedRef {
        name: "gscr",
        value: "\u{210A}",
        legacy: false,
    },
    NamedRef {
        name: "gsim",
        value: "\u{2273}",
        legacy: false,
    },
    NamedRef {
        name: "gsime",
        value: "\u{2A8E}",
        legacy: false,
    },
    NamedRef {
        name: "gsiml",
        value: "\u{2A90}",
        legacy: false,
    },
    NamedRef {
        name: "gt",
        value: "\u{3E}",
        legacy: true,
    },
    NamedRef {
        name: "gtcc",
        value: "\u{2AA7}",
        legacy: false,
    },
    NamedRef {
        name: "gtcir",
        value: "\u{2A7A}",
        legacy: false,
    },
    NamedRef {
        name: "gtdot",
        value: "\u{22D7}",
        legacy: false,
    },
    NamedRef {
        name: "gtlPar",
        value: "\u{2995}",
        legacy: false,
    },
    NamedRef {
        name: "gtquest",
        value: "\u{2A7C}",
        legacy: false,
    },
    NamedRef {
        name: "gtrapprox",
        value: "\u{2A86}",
        legacy: false,
    },
    NamedRef {
        name: "gtrarr",
        value: "\u{2978}",
        legacy: false,
    },
    NamedRef {
        name: "gtrdot",
        value: "\u{22D7}",
        legacy: false,
    },
    NamedRef {
        name: "gtreqless",
        value: "\u{22DB}",
        legacy: false,
    },
    NamedRef {
        name: "gtreqqless",
        value: "\u{2A8C}",
        legacy: false,
    },
    NamedRef {
        name: "gtrless",
        value: "\u{2277}",
        legacy: false,
    },
    NamedRef {
        name: "gtrsim",
        value: "\u{2273}",
        legacy: false,
    },
    NamedRef {
        name: "gvertneqq",
        value: "\u{2269}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "gvnE",
        value: "\u{2269}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "hArr",
        value: "\u{21D4}",
        legacy: false,
    },
    NamedRef {
        name: "hairsp",
        value: "\u{200A}",
        legacy: false,
    },
    NamedRef {
        name: "half",
        value: "\u{BD}",
        legacy: false,
    },
    NamedRef {
        name: "hamilt",
        value: "\u{210B}",
        legacy: false,
    },
    NamedRef {
        name: "hardcy",
        value: "\u{44A}",
        legacy: false,
    },
    NamedRef {
        name: "harr",
        value: "\u{2194}",
        legacy: false,
    },
    NamedRef {
        name: "harrcir",
        value: "\u{2948}",
        legacy: false,
    },
    NamedRef {
        name: "harrw",
        value: "\u{21AD}",
        legacy: false,
    },
    NamedRef {
        name: "hbar",
        value: "\u{210F}",
        legacy: false,
    },
    NamedRef {
        name: "hcirc",
        value: "\u{125}",
        legacy: false,
    },
    NamedRef {
        name: "hearts",
        value: "\u{2665}",
        legacy: false,
    },
    NamedRef {
        name: "heartsuit",
        value: "\u{2665}",
        legacy: false,
    },
    NamedRef {
        name: "hellip",
        value: "\u{2026}",
        legacy: false,
    },
    NamedRef {
        name: "hercon",
        value: "\u{22B9}",
        legacy: false,
    },
    NamedRef {
        name: "hfr",
        value: "\u{1D525}",
        legacy: false,
    },
    NamedRef {
        name: "hksearow",
        value: "\u{2925}",
        legacy: false,
    },
    NamedRef {
        name: "hkswarow",
        value: "\u{2926}",
        legacy: false,
    },
    NamedRef {
        name: "hoarr",
        value: "\u{21FF}",
        legacy: false,
    },
    NamedRef {
        name: "homtht",
        value: "\u{223B}",
        legacy: false,
    },
    NamedRef {
        name: "hookleftarrow",
        value: "\u{21A9}",
        legacy: false,
    },
    NamedRef {
        name: "hookrightarrow",
        value: "\u{21AA}",
        legacy: false,
    },
    NamedRef {
        name: "hopf",
        value: "\u{1D559}",
        legacy: false,
    },
    NamedRef {
        name: "horbar",
        value: "\u{2015}",
        legacy: false,
    },
    NamedRef {
        name: "hscr",
        value: "\u{1D4BD}",
        legacy: false,
    },
    NamedRef {
        name: "hslash",
        value: "\u{210F}",
        legacy: false,
    },
    NamedRef {
        name: "hstrok",
        value: "\u{127}",
        legacy: false,
    },
    NamedRef {
        name: "hybull",
        value: "\u{2043}",
        legacy: false,
    },
    NamedRef {
        name: "hyphen",
        value: "\u{2010}",
        legacy: false,
    },
    NamedRef {
        name: "iacute",
        value: "\u{ED}",
        legacy: true,
    },
    NamedRef {
        name: "ic",
        value: "\u{2063}",
        legacy: false,
    },
    NamedRef {
        name: "icirc",
        value: "\u{EE}",
        legacy: true,
    },
    NamedRef {
        name: "icy",
        value: "\u{438}",
        legacy: false,
    },
    NamedRef {
        name: "iecy",
        value: "\u{435}",
        legacy: false,
    },
    NamedRef {
        name: "iexcl",
        value: "\u{A1}",
        legacy: true,
    },
    NamedRef {
        name: "iff",
        value: "\u{21D4}",
        legacy: false,
    },
    NamedRef {
        name: "ifr",
        value: "\u{1D526}",
        legacy: false,
    },
    NamedRef {
        name: "igrave",
        value: "\u{EC}",
        legacy: true,
    },
    NamedRef {
        name: "ii",
        value: "\u{2148}",
        legacy: false,
    },
    NamedRef {
        name: "iiiint",
        value: "\u{2A0C}",
        legacy: false,
    },
    NamedRef {
        name: "iiint",
        value: "\u{222D}",
        legacy: false,
    },
    NamedRef {
        name: "iinfin",
        value: "\u{29DC}",
        legacy: false,
    },
    NamedRef {
        name: "iiota",
        value: "\u{2129}",
        legacy: false,
    },
    NamedRef {
        name: "ijlig",
        value: "\u{133}",
        legacy: false,
    },
    NamedRef {
        name: "imacr",
        value: "\u{12B}",
        legacy: false,
    },
    NamedRef {
        name: "image",
        value: "\u{2111}",
        legacy: false,
    },
    NamedRef {
        name: "imagline",
        value: "\u{2110}",
        legacy: false,
    },
    NamedRef {
        name: "imagpart",
        value: "\u{2111}",
        legacy: false,
    },
    NamedRef {
        name: "imath",
        value: "\u{131}",
        legacy: false,
    },
    NamedRef {
        name: "imof",
        value: "\u{22B7}",
        legacy: false,
    },
    NamedRef {
        name: "imped",
        value: "\u{1B5}",
        legacy: false,
    },
    NamedRef {
        name: "in",
        value: "\u{2208}",
        legacy: false,
    },
    NamedRef {
        name: "incare",
        value: "\u{2105}",
        legacy: false,
    },
    NamedRef {
        name: "infin",
        value: "\u{221E}",
        legacy: false,
    },
    NamedRef {
        name: "infintie",
        value: "\u{29DD}",
        legacy: false,
    },
    NamedRef {
        name: "inodot",
        value: "\u{131}",
        legacy: false,
    },
    NamedRef {
        name: "int",
        value: "\u{222B}",
        legacy: false,
    },
    NamedRef {
        name: "intcal",
        value: "\u{22BA}",
        legacy: false,
    },
    NamedRef {
        name: "integers",
        value: "\u{2124}",
        legacy: false,
    },
    NamedRef {
        name: "intercal",
        value: "\u{22BA}",
        legacy: false,
    },
    NamedRef {
        name: "intlarhk",
        value: "\u{2A17}",
        legacy: false,
    },
    NamedRef {
        name: "intprod",
        value: "\u{2A3C}",
        legacy: false,
    },
    NamedRef {
        name: "iocy",
        value: "\u{451}",
        legacy: false,
    },
    NamedRef {
        name: "iogon",
        value: "\u{12F}",
        legacy: false,
    },
    NamedRef {
        name: "iopf",
        value: "\u{1D55A}",
        legacy: false,
    },
    NamedRef {
        name: "iota",
        value: "\u{3B9}",
        legacy: false,
    },
    NamedRef {
        name: "iprod",
        value: "\u{2A3C}",
        legacy: false,
    },
    NamedRef {
        name: "iquest",
        value: "\u{BF}",
        legacy: true,
    },
    NamedRef {
        name: "iscr",
        value: "\u{1D4BE}",
        legacy: false,
    },
    NamedRef {
        name: "isin",
        value: "\u{2208}",
        legacy: false,
    },
    NamedRef {
        name: "isinE",
        value: "\u{22F9}",
        legacy: false,
    },
    NamedRef {
        name: "isindot",
        value: "\u{22F5}",
        legacy: false,
    },
    NamedRef {
        name: "isins",
        value: "\u{22F4}",
        legacy: false,
    },
    NamedRef {
        name: "isinsv",
        value: "\u{22F3}",
        legacy: false,
    },
    NamedRef {
        name: "isinv",
        value: "\u{2208}",
        legacy: false,
    },
    NamedRef {
        name: "it",
        value: "\u{2062}",
        legacy: false,
    },
    NamedRef {
        name: "itilde",
        value: "\u{129}",
        legacy: false,
    },
    NamedRef {
        name: "iukcy",
        value: "\u{456}",
        legacy: false,
    },
    NamedRef {
        name: "iuml",
        value: "\u{EF}",
        legacy: true,
    },
    NamedRef {
        name: "jcirc",
        value: "\u{135}",
        legacy: false,
    },
    NamedRef {
        name: "jcy",
        value: "\u{439}",
        legacy: false,
    },
    NamedRef {
        name: "jfr",
        value: "\u{1D527}",
        legacy: false,
    },
    NamedRef {
        name: "jmath",
        value: "\u{237}",
        legacy: false,
    },
    NamedRef {
        name: "jopf",
        value: "\u{1D55B}",
        legacy: false,
    },
    NamedRef {
        name: "jscr",
        value: "\u{1D4BF}",
        legacy: false,
    },
    NamedRef {
        name: "jsercy",
        value: "\u{458}",
        legacy: false,
    },
    NamedRef {
        name: "jukcy",
        value: "\u{454}",
        legacy: false,
    },
    NamedRef {
        name: "kappa",
        value: "\u{3BA}",
        legacy: false,
    },
    NamedRef {
        name: "kappav",
        value: "\u{3F0}",
        legacy: false,
    },
    NamedRef {
        name: "kcedil",
        value: "\u{137}",
        legacy: false,
    },
    NamedRef {
        name: "kcy",
        value: "\u{43A}",
        legacy: false,
    },
    NamedRef {
        name: "kfr",
        value: "\u{1D528}",
        legacy: false,
    },
    NamedRef {
        name: "kgreen",
        value: "\u{138}",
        legacy: false,
    },
    NamedRef {
        name: "khcy",
        value: "\u{445}",
        legacy: false,
    },
    NamedRef {
        name: "kjcy",
        value: "\u{45C}",
        legacy: false,
    },
    NamedRef {
        name: "kopf",
        value: "\u{1D55C}",
        legacy: false,
    },
    NamedRef {
        name: "kscr",
        value: "\u{1D4C0}",
        legacy: false,
    },
    NamedRef {
        name: "lAarr",
        value: "\u{21DA}",
        legacy: false,
    },
    NamedRef {
        name: "lArr",
        value: "\u{21D0}",
        legacy: false,
    },
    NamedRef {
        name: "lAtail",
        value: "\u{291B}",
        legacy: false,
    },
    NamedRef {
        name: "lBarr",
        value: "\u{290E}",
        legacy: false,
    },
    NamedRef {
        name: "lE",
        value: "\u{2266}",
        legacy: false,
    },
    NamedRef {
        name: "lEg",
        value: "\u{2A8B}",
        legacy: false,
    },
    NamedRef {
        name: "lHar",
        value: "\u{2962}",
        legacy: false,
    },
    NamedRef {
        name: "lacute",
        value: "\u{13A}",
        legacy: false,
    },
    NamedRef {
        name: "laemptyv",
        value: "\u{29B4}",
        legacy: false,
    },
    NamedRef {
        name: "lagran",
        value: "\u{2112}",
        legacy: false,
    },
    NamedRef {
        name: "lambda",
        value: "\u{3BB}",
        legacy: false,
    },
    NamedRef {
        name: "lang",
        value: "\u{27E8}",
        legacy: false,
    },
    NamedRef {
        name: "langd",
        value: "\u{2991}",
        legacy: false,
    },
    NamedRef {
        name: "langle",
        value: "\u{27E8}",
        legacy: false,
    },
    NamedRef {
        name: "lap",
        value: "\u{2A85}",
        legacy: false,
    },
    NamedRef {
        name: "laquo",
        value: "\u{AB}",
        legacy: true,
    },
    NamedRef {
        name: "larr",
        value: "\u{2190}",
        legacy: false,
    },
    NamedRef {
        name: "larrb",
        value: "\u{21E4}",
        legacy: false,
    },
    NamedRef {
        name: "larrbfs",
        value: "\u{291F}",
        legacy: false,
    },
    NamedRef {
        name: "larrfs",
        value: "\u{291D}",
        legacy: false,
    },
    NamedRef {
        name: "larrhk",
        value: "\u{21A9}",
        legacy: false,
    },
    NamedRef {
        name: "larrlp",
        value: "\u{21AB}",
        legacy: false,
    },
    NamedRef {
        name: "larrpl",
        value: "\u{2939}",
        legacy: false,
    },
    NamedRef {
        name: "larrsim",
        value: "\u{2973}",
        legacy: false,
    },
    NamedRef {
        name: "larrtl",
        value: "\u{21A2}",
        legacy: false,
    },
    NamedRef {
        name: "lat",
        value: "\u{2AAB}",
        legacy: false,
    },
    NamedRef {
        name: "latail",
        value: "\u{2919}",
        legacy: false,
    },
    NamedRef {
        name: "late",
        value: "\u{2AAD}",
        legacy: false,
    },
    NamedRef {
        name: "lates",
        value: "\u{2AAD}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "lbarr",
        value: "\u{290C}",
        legacy: false,
    },
    NamedRef {
        name: "lbbrk",
        value: "\u{2772}",
        legacy: false,
    },
    NamedRef {
        name: "lbrace",
        value: "\u{7B}",
        legacy: false,
    },
    NamedRef {
        name: "lbrack",
        value: "\u{5B}",
        legacy: false,
    },
    NamedRef {
        name: "lbrke",
        value: "\u{298B}",
        legacy: false,
    },
    NamedRef {
        name: "lbrksld",
        value: "\u{298F}",
        legacy: false,
    },
    NamedRef {
        name: "lbrkslu",
        value: "\u{298D}",
        legacy: false,
    },
    NamedRef {
        name: "lcaron",
        value: "\u{13E}",
        legacy: false,
    },
    NamedRef {
        name: "lcedil",
        value: "\u{13C}",
        legacy: false,
    },
    NamedRef {
        name: "lceil",
        value: "\u{2308}",
        legacy: false,
    },
    NamedRef {
        name: "lcub",
        value: "\u{7B}",
        legacy: false,
    },
    NamedRef {
        name: "lcy",
        value: "\u{43B}",
        legacy: false,
    },
    NamedRef {
        name: "ldca",
        value: "\u{2936}",
        legacy: false,
    },
    NamedRef {
        name: "ldquo",
        value: "\u{201C}",
        legacy: false,
    },
    NamedRef {
        name: "ldquor",
        value: "\u{201E}",
        legacy: false,
    },
    NamedRef {
        name: "ldrdhar",
        value: "\u{2967}",
        legacy: false,
    },
    NamedRef {
        name: "ldrushar",
        value: "\u{294B}",
        legacy: false,
    },
    NamedRef {
        name: "ldsh",
        value: "\u{21B2}",
        legacy: false,
    },
    NamedRef {
        name: "le",
        value: "\u{2264}",
        legacy: false,
    },
    NamedRef {
        name: "leftarrow",
        value: "\u{2190}",
        legacy: false,
    },
    NamedRef {
        name: "leftarrowtail",
        value: "\u{21A2}",
        legacy: false,
    },
    NamedRef {
        name: "leftharpoondown",
        value: "\u{21BD}",
        legacy: false,
    },
    NamedRef {
        name: "leftharpoonup",
        value: "\u{21BC}",
        legacy: false,
    },
    NamedRef {
        name: "leftleftarrows",
        value: "\u{21C7}",
        legacy: false,
    },
    NamedRef {
        name: "leftrightarrow",
        value: "\u{2194}",
        legacy: false,
    },
    NamedRef {
        name: "leftrightarrows",
        value: "\u{21C6}",
        legacy: false,
    },
    NamedRef {
        name: "leftrightharpoons",
        value: "\u{21CB}",
        legacy: false,
    },
    NamedRef {
        name: "leftrightsquigarrow",
        value: "\u{21AD}",
        legacy: false,
    },
    NamedRef {
        name: "leftthreetimes",
        value: "\u{22CB}",
        legacy: false,
    },
    NamedRef {
        name: "leg",
        value: "\u{22DA}",
        legacy: false,
    },
    NamedRef {
        name: "leq",
        value: "\u{2264}",
        legacy: false,
    },
    NamedRef {
        name: "leqq",
        value: "\u{2266}",
        legacy: false,
    },
    NamedRef {
        name: "leqslant",
        value: "\u{2A7D}",
        legacy: false,
    },
    NamedRef {
        name: "les",
        value: "\u{2A7D}",
        legacy: false,
    },
    NamedRef {
        name: "lescc",
        value: "\u{2AA8}",
        legacy: false,
    },
    NamedRef {
        name: "lesdot",
        value: "\u{2A7F}",
        legacy: false,
    },
    NamedRef {
        name: "lesdoto",
        value: "\u{2A81}",
        legacy: false,
    },
    NamedRef {
        name: "lesdotor",
        value: "\u{2A83}",
        legacy: false,
    },
    NamedRef {
        name: "lesg",
        value: "\u{22DA}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "lesges",
        value: "\u{2A93}",
        legacy: false,
    },
    NamedRef {
        name: "lessapprox",
        value: "\u{2A85}",
        legacy: false,
    },
    NamedRef {
        name: "lessdot",
        value: "\u{22D6}",
        legacy: false,
    },
    NamedRef {
        name: "lesseqgtr",
        value: "\u{22DA}",
        legacy: false,
    },
    NamedRef {
        name: "lesseqqgtr",
        value: "\u{2A8B}",
        legacy: false,
    },
    NamedRef {
        name: "lessgtr",
        value: "\u{2276}",
        legacy: false,
    },
    NamedRef {
        name: "lesssim",
        value: "\u{2272}",
        legacy: false,
    },
    NamedRef {
        name: "lfisht",
        value: "\u{297C}",
        legacy: false,
    },
    NamedRef {
        name: "lfloor",
        value: "\u{230A}",
        legacy: false,
    },
    NamedRef {
        name: "lfr",
        value: "\u{1D529}",
        legacy: false,
    },
    NamedRef {
        name: "lg",
        value: "\u{2276}",
        legacy: false,
    },
    NamedRef {
        name: "lgE",
        value: "\u{2A91}",
        legacy: false,
    },
    NamedRef {
        name: "lhard",
        value: "\u{21BD}",
        legacy: false,
    },
    NamedRef {
        name: "lharu",
        value: "\u{21BC}",
        legacy: false,
    },
    NamedRef {
        name: "lharul",
        value: "\u{296A}",
        legacy: false,
    },
    NamedRef {
        name: "lhblk",
        value: "\u{2584}",
        legacy: false,
    },
    NamedRef {
        name: "ljcy",
        value: "\u{459}",
        legacy: false,
    },
    NamedRef {
        name: "ll",
        value: "\u{226A}",
        legacy: false,
    },
    NamedRef {
        name: "llarr",
        value: "\u{21C7}",
        legacy: false,
    },
    NamedRef {
        name: "llcorner",
        value: "\u{231E}",
        legacy: false,
    },
    NamedRef {
        name: "llhard",
        value: "\u{296B}",
        legacy: false,
    },
    NamedRef {
        name: "lltri",
        value: "\u{25FA}",
        legacy: false,
    },
    NamedRef {
        name: "lmidot",
        value: "\u{140}",
        legacy: false,
    },
    NamedRef {
        name: "lmoust",
        value: "\u{23B0}",
        legacy: false,
    },
    NamedRef {
        name: "lmoustache",
        value: "\u{23B0}",
        legacy: false,
    },
    NamedRef {
        name: "lnE",
        value: "\u{2268}",
        legacy: false,
    },
    NamedRef {
        name: "lnap",
        value: "\u{2A89}",
        legacy: false,
    },
    NamedRef {
        name: "lnapprox",
        value: "\u{2A89}",
        legacy: false,
    },
    NamedRef {
        name: "lne",
        value: "\u{2A87}",
        legacy: false,
    },
    NamedRef {
        name: "lneq",
        value: "\u{2A87}",
        legacy: false,
    },
    NamedRef {
        name: "lneqq",
        value: "\u{2268}",
        legacy: false,
    },
    NamedRef {
        name: "lnsim",
        value: "\u{22E6}",
        legacy: false,
    },
    NamedRef {
        name: "loang",
        value: "\u{27EC}",
        legacy: false,
    },
    NamedRef {
        name: "loarr",
        value: "\u{21FD}",
        legacy: false,
    },
    NamedRef {
        name: "lobrk",
        value: "\u{27E6}",
        legacy: false,
    },
    NamedRef {
        name: "longleftarrow",
        value: "\u{27F5}",
        legacy: false,
    },
    NamedRef {
        name: "longleftrightarrow",
        value: "\u{27F7}",
        legacy: false,
    },
    NamedRef {
        name: "longmapsto",
        value: "\u{27FC}",
        legacy: false,
    },
    NamedRef {
        name: "longrightarrow",
        value: "\u{27F6}",
        legacy: false,
    },
    NamedRef {
        name: "looparrowleft",
        value: "\u{21AB}",
        legacy: false,
    },
    NamedRef {
        name: "looparrowright",
        value: "\u{21AC}",
        legacy: false,
    },
    NamedRef {
        name: "lopar",
        value: "\u{2985}",
        legacy: false,
    },
    NamedRef {
        name: "lopf",
        value: "\u{1D55D}",
        legacy: false,
    },
    NamedRef {
        name: "loplus",
        value: "\u{2A2D}",
        legacy: false,
    },
    NamedRef {
        name: "lotimes",
        value: "\u{2A34}",
        legacy: false,
    },
    NamedRef {
        name: "lowast",
        value: "\u{2217}",
        legacy: false,
    },
    NamedRef {
        name: "lowbar",
        value: "\u{5F}",
        legacy: false,
    },
    NamedRef {
        name: "loz",
        value: "\u{25CA}",
        legacy: false,
    },
    NamedRef {
        name: "lozenge",
        value: "\u{25CA}",
        legacy: false,
    },
    NamedRef {
        name: "lozf",
        value: "\u{29EB}",
        legacy: false,
    },
    NamedRef {
        name: "lpar",
        value: "\u{28}",
        legacy: false,
    },
    NamedRef {
        name: "lparlt",
        value: "\u{2993}",
        legacy: false,
    },
    NamedRef {
        name: "lrarr",
        value: "\u{21C6}",
        legacy: false,
    },
    NamedRef {
        name: "lrcorner",
        value: "\u{231F}",
        legacy: false,
    },
    NamedRef {
        name: "lrhar",
        value: "\u{21CB}",
        legacy: false,
    },
    NamedRef {
        name: "lrhard",
        value: "\u{296D}",
        legacy: false,
    },
    NamedRef {
        name: "lrm",
        value: "\u{200E}",
        legacy: false,
    },
    NamedRef {
        name: "lrtri",
        value: "\u{22BF}",
        legacy: false,
    },
    NamedRef {
        name: "lsaquo",
        value: "\u{2039}",
        legacy: false,
    },
    NamedRef {
        name: "lscr",
        value: "\u{1D4C1}",
        legacy: false,
    },
    NamedRef {
        name: "lsh",
        value: "\u{21B0}",
        legacy: false,
    },
    NamedRef {
        name: "lsim",
        value: "\u{2272}",
        legacy: false,
    },
    NamedRef {
        name: "lsime",
        value: "\u{2A8D}",
        legacy: false,
    },
    NamedRef {
        name: "lsimg",
        value: "\u{2A8F}",
        legacy: false,
    },
    NamedRef {
        name: "lsqb",
        value: "\u{5B}",
        legacy: false,
    },
    NamedRef {
        name: "lsquo",
        value: "\u{2018}",
        legacy: false,
    },
    NamedRef {
        name: "lsquor",
        value: "\u{201A}",
        legacy: false,
    },
    NamedRef {
        name: "lstrok",
        value: "\u{142}",
        legacy: false,
    },
    NamedRef {
        name: "lt",
        value: "\u{3C}",
        legacy: true,
    },
    NamedRef {
        name: "ltcc",
        value: "\u{2AA6}",
        legacy: false,
    },
    NamedRef {
        name: "ltcir",
        value: "\u{2A79}",
        legacy: false,
    },
    NamedRef {
        name: "ltdot",
        value: "\u{22D6}",
        legacy: false,
    },
    NamedRef {
        name: "lthree",
        value: "\u{22CB}",
        legacy: false,
    },
    NamedRef {
        name: "ltimes",
        value: "\u{22C9}",
        legacy: false,
    },
    NamedRef {
        name: "ltlarr",
        value: "\u{2976}",
        legacy: false,
    },
    NamedRef {
        name: "ltquest",
        value: "\u{2A7B}",
        legacy: false,
    },
    NamedRef {
        name: "ltrPar",
        value: "\u{2996}",
        legacy: false,
    },
    NamedRef {
        name: "ltri",
        value: "\u{25C3}",
        legacy: false,
    },
    NamedRef {
        name: "ltrie",
        value: "\u{22B4}",
        legacy: false,
    },
    NamedRef {
        name: "ltrif",
        value: "\u{25C2}",
        legacy: false,
    },
    NamedRef {
        name: "lurdshar",
        value: "\u{294A}",
        legacy: false,
    },
    NamedRef {
        name: "luruhar",
        value: "\u{2966}",
        legacy: false,
    },
    NamedRef {
        name: "lvertneqq",
        value: "\u{2268}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "lvnE",
        value: "\u{2268}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "mDDot",
        value: "\u{223A}",
        legacy: false,
    },
    NamedRef {
        name: "macr",
        value: "\u{AF}",
        legacy: true,
    },
    NamedRef {
        name: "male",
        value: "\u{2642}",
        legacy: false,
    },
    NamedRef {
        name: "malt",
        value: "\u{2720}",
        legacy: false,
    },
    NamedRef {
        name: "maltese",
        value: "\u{2720}",
        legacy: false,
    },
    NamedRef {
        name: "map",
        value: "\u{21A6}",
        legacy: false,
    },
    NamedRef {
        name: "mapsto",
        value: "\u{21A6}",
        legacy: false,
    },
    NamedRef {
        name: "mapstodown",
        value: "\u{21A7}",
        legacy: false,
    },
    NamedRef {
        name: "mapstoleft",
        value: "\u{21A4}",
        legacy: false,
    },
    NamedRef {
        name: "mapstoup",
        value: "\u{21A5}",
        legacy: false,
    },
    NamedRef {
        name: "marker",
        value: "\u{25AE}",
        legacy: false,
    },
    NamedRef {
        name: "mcomma",
        value: "\u{2A29}",
        legacy: false,
    },
    NamedRef {
        name: "mcy",
        value: "\u{43C}",
        legacy: false,
    },
    NamedRef {
        name: "mdash",
        value: "\u{2014}",
        legacy: false,
    },
    NamedRef {
        name: "measuredangle",
        value: "\u{2221}",
        legacy: false,
    },
    NamedRef {
        name: "mfr",
        value: "\u{1D52A}",
        legacy: false,
    },
    NamedRef {
        name: "mho",
        value: "\u{2127}",
        legacy: false,
    },
    NamedRef {
        name: "micro",
        value: "\u{B5}",
        legacy: true,
    },
    NamedRef {
        name: "mid",
        value: "\u{2223}",
        legacy: false,
    },
    NamedRef {
        name: "midast",
        value: "\u{2A}",
        legacy: false,
    },
    NamedRef {
        name: "midcir",
        value: "\u{2AF0}",
        legacy: false,
    },
    NamedRef {
        name: "middot",
        value: "\u{B7}",
        legacy: true,
    },
    NamedRef {
        name: "minus",
        value: "\u{2212}",
        legacy: false,
    },
    NamedRef {
        name: "minusb",
        value: "\u{229F}",
        legacy: false,
    },
    NamedRef {
        name: "minusd",
        value: "\u{2238}",
        legacy: false,
    },
    NamedRef {
        name: "minusdu",
        value: "\u{2A2A}",
        legacy: false,
    },
    NamedRef {
        name: "mlcp",
        value: "\u{2ADB}",
        legacy: false,
    },
    NamedRef {
        name: "mldr",
        value: "\u{2026}",
        legacy: false,
    },
    NamedRef {
        name: "mnplus",
        value: "\u{2213}",
        legacy: false,
    },
    NamedRef {
        name: "models",
        value: "\u{22A7}",
        legacy: false,
    },
    NamedRef {
        name: "mopf",
        value: "\u{1D55E}",
        legacy: false,
    },
    NamedRef {
        name: "mp",
        value: "\u{2213}",
        legacy: false,
    },
    NamedRef {
        name: "mscr",
        value: "\u{1D4C2}",
        legacy: false,
    },
    NamedRef {
        name: "mstpos",
        value: "\u{223E}",
        legacy: false,
    },
    NamedRef {
        name: "mu",
        value: "\u{3BC}",
        legacy: false,
    },
    NamedRef {
        name: "multimap",
        value: "\u{22B8}",
        legacy: false,
    },
    NamedRef {
        name: "mumap",
        value: "\u{22B8}",
        legacy: false,
    },
    NamedRef {
        name: "nGg",
        value: "\u{22D9}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nGt",
        value: "\u{226B}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nGtv",
        value: "\u{226B}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nLeftarrow",
        value: "\u{21CD}",
        legacy: false,
    },
    NamedRef {
        name: "nLeftrightarrow",
        value: "\u{21CE}",
        legacy: false,
    },
    NamedRef {
        name: "nLl",
        value: "\u{22D8}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nLt",
        value: "\u{226A}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nLtv",
        value: "\u{226A}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nRightarrow",
        value: "\u{21CF}",
        legacy: false,
    },
    NamedRef {
        name: "nVDash",
        value: "\u{22AF}",
        legacy: false,
    },
    NamedRef {
        name: "nVdash",
        value: "\u{22AE}",
        legacy: false,
    },
    NamedRef {
        name: "nabla",
        value: "\u{2207}",
        legacy: false,
    },
    NamedRef {
        name: "nacute",
        value: "\u{144}",
        legacy: false,
    },
    NamedRef {
        name: "nang",
        value: "\u{2220}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nap",
        value: "\u{2249}",
        legacy: false,
    },
    NamedRef {
        name: "napE",
        value: "\u{2A70}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "napid",
        value: "\u{224B}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "napos",
        value: "\u{149}",
        legacy: false,
    },
    NamedRef {
        name: "napprox",
        value: "\u{2249}",
        legacy: false,
    },
    NamedRef {
        name: "natur",
        value: "\u{266E}",
        legacy: false,
    },
    NamedRef {
        name: "natural",
        value: "\u{266E}",
        legacy: false,
    },
    NamedRef {
        name: "naturals",
        value: "\u{2115}",
        legacy: false,
    },
    NamedRef {
        name: "nbsp",
        value: "\u{A0}",
        legacy: true,
    },
    NamedRef {
        name: "nbump",
        value: "\u{224E}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nbumpe",
        value: "\u{224F}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "ncap",
        value: "\u{2A43}",
        legacy: false,
    },
    NamedRef {
        name: "ncaron",
        value: "\u{148}",
        legacy: false,
    },
    NamedRef {
        name: "ncedil",
        value: "\u{146}",
        legacy: false,
    },
    NamedRef {
        name: "ncong",
        value: "\u{2247}",
        legacy: false,
    },
    NamedRef {
        name: "ncongdot",
        value: "\u{2A6D}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "ncup",
        value: "\u{2A42}",
        legacy: false,
    },
    NamedRef {
        name: "ncy",
        value: "\u{43D}",
        legacy: false,
    },
    NamedRef {
        name: "ndash",
        value: "\u{2013}",
        legacy: false,
    },
    NamedRef {
        name: "ne",
        value: "\u{2260}",
        legacy: false,
    },
    NamedRef {
        name: "neArr",
        value: "\u{21D7}",
        legacy: false,
    },
    NamedRef {
        name: "nearhk",
        value: "\u{2924}",
        legacy: false,
    },
    NamedRef {
        name: "nearr",
        value: "\u{2197}",
        legacy: false,
    },
    NamedRef {
        name: "nearrow",
        value: "\u{2197}",
        legacy: false,
    },
    NamedRef {
        name: "nedot",
        value: "\u{2250}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nequiv",
        value: "\u{2262}",
        legacy: false,
    },
    NamedRef {
        name: "nesear",
        value: "\u{2928}",
        legacy: false,
    },
    NamedRef {
        name: "nesim",
        value: "\u{2242}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nexist",
        value: "\u{2204}",
        legacy: false,
    },
    NamedRef {
        name: "nexists",
        value: "\u{2204}",
        legacy: false,
    },
    NamedRef {
        name: "nfr",
        value: "\u{1D52B}",
        legacy: false,
    },
    NamedRef {
        name: "ngE",
        value: "\u{2267}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nge",
        value: "\u{2271}",
        legacy: false,
    },
    NamedRef {
        name: "ngeq",
        value: "\u{2271}",
        legacy: false,
    },
    NamedRef {
        name: "ngeqq",
        value: "\u{2267}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "ngeqslant",
        value: "\u{2A7E}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nges",
        value: "\u{2A7E}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "ngsim",
        value: "\u{2275}",
        legacy: false,
    },
    NamedRef {
        name: "ngt",
        value: "\u{226F}",
        legacy: false,
    },
    NamedRef {
        name: "ngtr",
        value: "\u{226F}",
        legacy: false,
    },
    NamedRef {
        name: "nhArr",
        value: "\u{21CE}",
        legacy: false,
    },
    NamedRef {
        name: "nharr",
        value: "\u{21AE}",
        legacy: false,
    },
    NamedRef {
        name: "nhpar",
        value: "\u{2AF2}",
        legacy: false,
    },
    NamedRef {
        name: "ni",
        value: "\u{220B}",
        legacy: false,
    },
    NamedRef {
        name: "nis",
        value: "\u{22FC}",
        legacy: false,
    },
    NamedRef {
        name: "nisd",
        value: "\u{22FA}",
        legacy: false,
    },
    NamedRef {
        name: "niv",
        value: "\u{220B}",
        legacy: false,
    },
    NamedRef {
        name: "njcy",
        value: "\u{45A}",
        legacy: false,
    },
    NamedRef {
        name: "nlArr",
        value: "\u{21CD}",
        legacy: false,
    },
    NamedRef {
        name: "nlE",
        value: "\u{2266}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nlarr",
        value: "\u{219A}",
        legacy: false,
    },
    NamedRef {
        name: "nldr",
        value: "\u{2025}",
        legacy: false,
    },
    NamedRef {
        name: "nle",
        value: "\u{2270}",
        legacy: false,
    },
    NamedRef {
        name: "nleftarrow",
        value: "\u{219A}",
        legacy: false,
    },
    NamedRef {
        name: "nleftrightarrow",
        value: "\u{21AE}",
        legacy: false,
    },
    NamedRef {
        name: "nleq",
        value: "\u{2270}",
        legacy: false,
    },
    NamedRef {
        name: "nleqq",
        value: "\u{2266}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nleqslant",
        value: "\u{2A7D}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nles",
        value: "\u{2A7D}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nless",
        value: "\u{226E}",
        legacy: false,
    },
    NamedRef {
        name: "nlsim",
        value: "\u{2274}",
        legacy: false,
    },
    NamedRef {
        name: "nlt",
        value: "\u{226E}",
        legacy: false,
    },
    NamedRef {
        name: "nltri",
        value: "\u{22EA}",
        legacy: false,
    },
    NamedRef {
        name: "nltrie",
        value: "\u{22EC}",
        legacy: false,
    },
    NamedRef {
        name: "nmid",
        value: "\u{2224}",
        legacy: false,
    },
    NamedRef {
        name: "nopf",
        value: "\u{1D55F}",
        legacy: false,
    },
    NamedRef {
        name: "not",
        value: "\u{AC}",
        legacy: true,
    },
    NamedRef {
        name: "notin",
        value: "\u{2209}",
        legacy: false,
    },
    NamedRef {
        name: "notinE",
        value: "\u{22F9}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "notindot",
        value: "\u{22F5}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "notinva",
        value: "\u{2209}",
        legacy: false,
    },
    NamedRef {
        name: "notinvb",
        value: "\u{22F7}",
        legacy: false,
    },
    NamedRef {
        name: "notinvc",
        value: "\u{22F6}",
        legacy: false,
    },
    NamedRef {
        name: "notni",
        value: "\u{220C}",
        legacy: false,
    },
    NamedRef {
        name: "notniva",
        value: "\u{220C}",
        legacy: false,
    },
    NamedRef {
        name: "notnivb",
        value: "\u{22FE}",
        legacy: false,
    },
    NamedRef {
        name: "notnivc",
        value: "\u{22FD}",
        legacy: false,
    },
    NamedRef {
        name: "npar",
        value: "\u{2226}",
        legacy: false,
    },
    NamedRef {
        name: "nparallel",
        value: "\u{2226}",
        legacy: false,
    },
    NamedRef {
        name: "nparsl",
        value: "\u{2AFD}\u{20E5}",
        legacy: false,
    },
    NamedRef {
        name: "npart",
        value: "\u{2202}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "npolint",
        value: "\u{2A14}",
        legacy: false,
    },
    NamedRef {
        name: "npr",
        value: "\u{2280}",
        legacy: false,
    },
    NamedRef {
        name: "nprcue",
        value: "\u{22E0}",
        legacy: false,
    },
    NamedRef {
        name: "npre",
        value: "\u{2AAF}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nprec",
        value: "\u{2280}",
        legacy: false,
    },
    NamedRef {
        name: "npreceq",
        value: "\u{2AAF}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nrArr",
        value: "\u{21CF}",
        legacy: false,
    },
    NamedRef {
        name: "nrarr",
        value: "\u{219B}",
        legacy: false,
    },
    NamedRef {
        name: "nrarrc",
        value: "\u{2933}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nrarrw",
        value: "\u{219D}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nrightarrow",
        value: "\u{219B}",
        legacy: false,
    },
    NamedRef {
        name: "nrtri",
        value: "\u{22EB}",
        legacy: false,
    },
    NamedRef {
        name: "nrtrie",
        value: "\u{22ED}",
        legacy: false,
    },
    NamedRef {
        name: "nsc",
        value: "\u{2281}",
        legacy: false,
    },
    NamedRef {
        name: "nsccue",
        value: "\u{22E1}",
        legacy: false,
    },
    NamedRef {
        name: "nsce",
        value: "\u{2AB0}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nscr",
        value: "\u{1D4C3}",
        legacy: false,
    },
    NamedRef {
        name: "nshortmid",
        value: "\u{2224}",
        legacy: false,
    },
    NamedRef {
        name: "nshortparallel",
        value: "\u{2226}",
        legacy: false,
    },
    NamedRef {
        name: "nsim",
        value: "\u{2241}",
        legacy: false,
    },
    NamedRef {
        name: "nsime",
        value: "\u{2244}",
        legacy: false,
    },
    NamedRef {
        name: "nsimeq",
        value: "\u{2244}",
        legacy: false,
    },
    NamedRef {
        name: "nsmid",
        value: "\u{2224}",
        legacy: false,
    },
    NamedRef {
        name: "nspar",
        value: "\u{2226}",
        legacy: false,
    },
    NamedRef {
        name: "nsqsube",
        value: "\u{22E2}",
        legacy: false,
    },
    NamedRef {
        name: "nsqsupe",
        value: "\u{22E3}",
        legacy: false,
    },
    NamedRef {
        name: "nsub",
        value: "\u{2284}",
        legacy: false,
    },
    NamedRef {
        name: "nsubE",
        value: "\u{2AC5}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nsube",
        value: "\u{2288}",
        legacy: false,
    },
    NamedRef {
        name: "nsubset",
        value: "\u{2282}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nsubseteq",
        value: "\u{2288}",
        legacy: false,
    },
    NamedRef {
        name: "nsubseteqq",
        value: "\u{2AC5}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nsucc",
        value: "\u{2281}",
        legacy: false,
    },
    NamedRef {
        name: "nsucceq",
        value: "\u{2AB0}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nsup",
        value: "\u{2285}",
        legacy: false,
    },
    NamedRef {
        name: "nsupE",
        value: "\u{2AC6}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "nsupe",
        value: "\u{2289}",
        legacy: false,
    },
    NamedRef {
        name: "nsupset",
        value: "\u{2283}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nsupseteq",
        value: "\u{2289}",
        legacy: false,
    },
    NamedRef {
        name: "nsupseteqq",
        value: "\u{2AC6}\u{338}",
        legacy: false,
    },
    NamedRef {
        name: "ntgl",
        value: "\u{2279}",
        legacy: false,
    },
    NamedRef {
        name: "ntilde",
        value: "\u{F1}",
        legacy: true,
    },
    NamedRef {
        name: "ntlg",
        value: "\u{2278}",
        legacy: false,
    },
    NamedRef {
        name: "ntriangleleft",
        value: "\u{22EA}",
        legacy: false,
    },
    NamedRef {
        name: "ntrianglelefteq",
        value: "\u{22EC}",
        legacy: false,
    },
    NamedRef {
        name: "ntriangleright",
        value: "\u{22EB}",
        legacy: false,
    },
    NamedRef {
        name: "ntrianglerighteq",
        value: "\u{22ED}",
        legacy: false,
    },
    NamedRef {
        name: "nu",
        value: "\u{3BD}",
        legacy: false,
    },
    NamedRef {
        name: "num",
        value: "\u{23}",
        legacy: false,
    },
    NamedRef {
        name: "numero",
        value: "\u{2116}",
        legacy: false,
    },
    NamedRef {
        name: "numsp",
        value: "\u{2007}",
        legacy: false,
    },
    NamedRef {
        name: "nvDash",
        value: "\u{22AD}",
        legacy: false,
    },
    NamedRef {
        name: "nvHarr",
        value: "\u{2904}",
        legacy: false,
    },
    NamedRef {
        name: "nvap",
        value: "\u{224D}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nvdash",
        value: "\u{22AC}",
        legacy: false,
    },
    NamedRef {
        name: "nvge",
        value: "\u{2265}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nvgt",
        value: "\u{3E}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nvinfin",
        value: "\u{29DE}",
        legacy: false,
    },
    NamedRef {
        name: "nvlArr",
        value: "\u{2902}",
        legacy: false,
    },
    NamedRef {
        name: "nvle",
        value: "\u{2264}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nvlt",
        value: "\u{3C}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nvltrie",
        value: "\u{22B4}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nvrArr",
        value: "\u{2903}",
        legacy: false,
    },
    NamedRef {
        name: "nvrtrie",
        value: "\u{22B5}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nvsim",
        value: "\u{223C}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "nwArr",
        value: "\u{21D6}",
        legacy: false,
    },
    NamedRef {
        name: "nwarhk",
        value: "\u{2923}",
        legacy: false,
    },
    NamedRef {
        name: "nwarr",
        value: "\u{2196}",
        legacy: false,
    },
    NamedRef {
        name: "nwarrow",
        value: "\u{2196}",
        legacy: false,
    },
    NamedRef {
        name: "nwnear",
        value: "\u{2927}",
        legacy: false,
    },
    NamedRef {
        name: "oS",
        value: "\u{24C8}",
        legacy: false,
    },
    NamedRef {
        name: "oacute",
        value: "\u{F3}",
        legacy: true,
    },
    NamedRef {
        name: "oast",
        value: "\u{229B}",
        legacy: false,
    },
    NamedRef {
        name: "ocir",
        value: "\u{229A}",
        legacy: false,
    },
    NamedRef {
        name: "ocirc",
        value: "\u{F4}",
        legacy: true,
    },
    NamedRef {
        name: "ocy",
        value: "\u{43E}",
        legacy: false,
    },
    NamedRef {
        name: "odash",
        value: "\u{229D}",
        legacy: false,
    },
    NamedRef {
        name: "odblac",
        value: "\u{151}",
        legacy: false,
    },
    NamedRef {
        name: "odiv",
        value: "\u{2A38}",
        legacy: false,
    },
    NamedRef {
        name: "odot",
        value: "\u{2299}",
        legacy: false,
    },
    NamedRef {
        name: "odsold",
        value: "\u{29BC}",
        legacy: false,
    },
    NamedRef {
        name: "oelig",
        value: "\u{153}",
        legacy: false,
    },
    NamedRef {
        name: "ofcir",
        value: "\u{29BF}",
        legacy: false,
    },
    NamedRef {
        name: "ofr",
        value: "\u{1D52C}",
        legacy: false,
    },
    NamedRef {
        name: "ogon",
        value: "\u{2DB}",
        legacy: false,
    },
    NamedRef {
        name: "ograve",
        value: "\u{F2}",
        legacy: true,
    },
    NamedRef {
        name: "ogt",
        value: "\u{29C1}",
        legacy: false,
    },
    NamedRef {
        name: "ohbar",
        value: "\u{29B5}",
        legacy: false,
    },
    NamedRef {
        name: "ohm",
        value: "\u{3A9}",
        legacy: false,
    },
    NamedRef {
        name: "oint",
        value: "\u{222E}",
        legacy: false,
    },
    NamedRef {
        name: "olarr",
        value: "\u{21BA}",
        legacy: false,
    },
    NamedRef {
        name: "olcir",
        value: "\u{29BE}",
        legacy: false,
    },
    NamedRef {
        name: "olcross",
        value: "\u{29BB}",
        legacy: false,
    },
    NamedRef {
        name: "oline",
        value: "\u{203E}",
        legacy: false,
    },
    NamedRef {
        name: "olt",
        value: "\u{29C0}",
        legacy: false,
    },
    NamedRef {
        name: "omacr",
        value: "\u{14D}",
        legacy: false,
    },
    NamedRef {
        name: "omega",
        value: "\u{3C9}",
        legacy: false,
    },
    NamedRef {
        name: "omicron",
        value: "\u{3BF}",
        legacy: false,
    },
    NamedRef {
        name: "omid",
        value: "\u{29B6}",
        legacy: false,
    },
    NamedRef {
        name: "ominus",
        value: "\u{2296}",
        legacy: false,
    },
    NamedRef {
        name: "oopf",
        value: "\u{1D560}",
        legacy: false,
    },
    NamedRef {
        name: "opar",
        value: "\u{29B7}",
        legacy: false,
    },
    NamedRef {
        name: "operp",
        value: "\u{29B9}",
        legacy: false,
    },
    NamedRef {
        name: "oplus",
        value: "\u{2295}",
        legacy: false,
    },
    NamedRef {
        name: "or",
        value: "\u{2228}",
        legacy: false,
    },
    NamedRef {
        name: "orarr",
        value: "\u{21BB}",
        legacy: false,
    },
    NamedRef {
        name: "ord",
        value: "\u{2A5D}",
        legacy: false,
    },
    NamedRef {
        name: "order",
        value: "\u{2134}",
        legacy: false,
    },
    NamedRef {
        name: "orderof",
        value: "\u{2134}",
        legacy: false,
    },
    NamedRef {
        name: "ordf",
        value: "\u{AA}",
        legacy: true,
    },
    NamedRef {
        name: "ordm",
        value: "\u{BA}",
        legacy: true,
    },
    NamedRef {
        name: "origof",
        value: "\u{22B6}",
        legacy: false,
    },
    NamedRef {
        name: "oror",
        value: "\u{2A56}",
        legacy: false,
    },
    NamedRef {
        name: "orslope",
        value: "\u{2A57}",
        legacy: false,
    },
    NamedRef {
        name: "orv",
        value: "\u{2A5B}",
        legacy: false,
    },
    NamedRef {
        name: "oscr",
        value: "\u{2134}",
        legacy: false,
    },
    NamedRef {
        name: "oslash",
        value: "\u{F8}",
        legacy: true,
    },
    NamedRef {
        name: "osol",
        value: "\u{2298}",
        legacy: false,
    },
    NamedRef {
        name: "otilde",
        value: "\u{F5}",
        legacy: true,
    },
    NamedRef {
        name: "otimes",
        value: "\u{2297}",
        legacy: false,
    },
    NamedRef {
        name: "otimesas",
        value: "\u{2A36}",
        legacy: false,
    },
    NamedRef {
        name: "ouml",
        value: "\u{F6}",
        legacy: true,
    },
    NamedRef {
        name: "ovbar",
        value: "\u{233D}",
        legacy: false,
    },
    NamedRef {
        name: "par",
        value: "\u{2225}",
        legacy: false,
    },
    NamedRef {
        name: "para",
        value: "\u{B6}",
        legacy: true,
    },
    NamedRef {
        name: "parallel",
        value: "\u{2225}",
        legacy: false,
    },
    NamedRef {
        name: "parsim",
        value: "\u{2AF3}",
        legacy: false,
    },
    NamedRef {
        name: "parsl",
        value: "\u{2AFD}",
        legacy: false,
    },
    NamedRef {
        name: "part",
        value: "\u{2202}",
        legacy: false,
    },
    NamedRef {
        name: "pcy",
        value: "\u{43F}",
        legacy: false,
    },
    NamedRef {
        name: "percnt",
        value: "\u{25}",
        legacy: false,
    },
    NamedRef {
        name: "period",
        value: "\u{2E}",
        legacy: false,
    },
    NamedRef {
        name: "permil",
        value: "\u{2030}",
        legacy: false,
    },
    NamedRef {
        name: "perp",
        value: "\u{22A5}",
        legacy: false,
    },
    NamedRef {
        name: "pertenk",
        value: "\u{2031}",
        legacy: false,
    },
    NamedRef {
        name: "pfr",
        value: "\u{1D52D}",
        legacy: false,
    },
    NamedRef {
        name: "phi",
        value: "\u{3C6}",
        legacy: false,
    },
    NamedRef {
        name: "phiv",
        value: "\u{3D5}",
        legacy: false,
    },
    NamedRef {
        name: "phmmat",
        value: "\u{2133}",
        legacy: false,
    },
    NamedRef {
        name: "phone",
        value: "\u{260E}",
        legacy: false,
    },
    NamedRef {
        name: "pi",
        value: "\u{3C0}",
        legacy: false,
    },
    NamedRef {
        name: "pitchfork",
        value: "\u{22D4}",
        legacy: false,
    },
    NamedRef {
        name: "piv",
        value: "\u{3D6}",
        legacy: false,
    },
    NamedRef {
        name: "planck",
        value: "\u{210F}",
        legacy: false,
    },
    NamedRef {
        name: "planckh",
        value: "\u{210E}",
        legacy: false,
    },
    NamedRef {
        name: "plankv",
        value: "\u{210F}",
        legacy: false,
    },
    NamedRef {
        name: "plus",
        value: "\u{2B}",
        legacy: false,
    },
    NamedRef {
        name: "plusacir",
        value: "\u{2A23}",
        legacy: false,
    },
    NamedRef {
        name: "plusb",
        value: "\u{229E}",
        legacy: false,
    },
    NamedRef {
        name: "pluscir",
        value: "\u{2A22}",
        legacy: false,
    },
    NamedRef {
        name: "plusdo",
        value: "\u{2214}",
        legacy: false,
    },
    NamedRef {
        name: "plusdu",
        value: "\u{2A25}",
        legacy: false,
    },
    NamedRef {
        name: "pluse",
        value: "\u{2A72}",
        legacy: false,
    },
    NamedRef {
        name: "plusmn",
        value: "\u{B1}",
        legacy: true,
    },
    NamedRef {
        name: "plussim",
        value: "\u{2A26}",
        legacy: false,
    },
    NamedRef {
        name: "plustwo",
        value: "\u{2A27}",
        legacy: false,
    },
    NamedRef {
        name: "pm",
        value: "\u{B1}",
        legacy: false,
    },
    NamedRef {
        name: "pointint",
        value: "\u{2A15}",
        legacy: false,
    },
    NamedRef {
        name: "popf",
        value: "\u{1D561}",
        legacy: false,
    },
    NamedRef {
        name: "pound",
        value: "\u{A3}",
        legacy: true,
    },
    NamedRef {
        name: "pr",
        value: "\u{227A}",
        legacy: false,
    },
    NamedRef {
        name: "prE",
        value: "\u{2AB3}",
        legacy: false,
    },
    NamedRef {
        name: "prap",
        value: "\u{2AB7}",
        legacy: false,
    },
    NamedRef {
        name: "prcue",
        value: "\u{227C}",
        legacy: false,
    },
    NamedRef {
        name: "pre",
        value: "\u{2AAF}",
        legacy: false,
    },
    NamedRef {
        name: "prec",
        value: "\u{227A}",
        legacy: false,
    },
    NamedRef {
        name: "precapprox",
        value: "\u{2AB7}",
        legacy: false,
    },
    NamedRef {
        name: "preccurlyeq",
        value: "\u{227C}",
        legacy: false,
    },
    NamedRef {
        name: "preceq",
        value: "\u{2AAF}",
        legacy: false,
    },
    NamedRef {
        name: "precnapprox",
        value: "\u{2AB9}",
        legacy: false,
    },
    NamedRef {
        name: "precneqq",
        value: "\u{2AB5}",
        legacy: false,
    },
    NamedRef {
        name: "precnsim",
        value: "\u{22E8}",
        legacy: false,
    },
    NamedRef {
        name: "precsim",
        value: "\u{227E}",
        legacy: false,
    },
    NamedRef {
        name: "prime",
        value: "\u{2032}",
        legacy: false,
    },
    NamedRef {
        name: "primes",
        value: "\u{2119}",
        legacy: false,
    },
    NamedRef {
        name: "prnE",
        value: "\u{2AB5}",
        legacy: false,
    },
    NamedRef {
        name: "prnap",
        value: "\u{2AB9}",
        legacy: false,
    },
    NamedRef {
        name: "prnsim",
        value: "\u{22E8}",
        legacy: false,
    },
    NamedRef {
        name: "prod",
        value: "\u{220F}",
        legacy: false,
    },
    NamedRef {
        name: "profalar",
        value: "\u{232E}",
        legacy: false,
    },
    NamedRef {
        name: "profline",
        value: "\u{2312}",
        legacy: false,
    },
    NamedRef {
        name: "profsurf",
        value: "\u{2313}",
        legacy: false,
    },
    NamedRef {
        name: "prop",
        value: "\u{221D}",
        legacy: false,
    },
    NamedRef {
        name: "propto",
        value: "\u{221D}",
        legacy: false,
    },
    NamedRef {
        name: "prsim",
        value: "\u{227E}",
        legacy: false,
    },
    NamedRef {
        name: "prurel",
        value: "\u{22B0}",
        legacy: false,
    },
    NamedRef {
        name: "pscr",
        value: "\u{1D4C5}",
        legacy: false,
    },
    NamedRef {
        name: "psi",
        value: "\u{3C8}",
        legacy: false,
    },
    NamedRef {
        name: "puncsp",
        value: "\u{2008}",
        legacy: false,
    },
    NamedRef {
        name: "qfr",
        value: "\u{1D52E}",
        legacy: false,
    },
    NamedRef {
        name: "qint",
        value: "\u{2A0C}",
        legacy: false,
    },
    NamedRef {
        name: "qopf",
        value: "\u{1D562}",
        legacy: false,
    },
    NamedRef {
        name: "qprime",
        value: "\u{2057}",
        legacy: false,
    },
    NamedRef {
        name: "qscr",
        value: "\u{1D4C6}",
        legacy: false,
    },
    NamedRef {
        name: "quaternions",
        value: "\u{210D}",
        legacy: false,
    },
    NamedRef {
        name: "quatint",
        value: "\u{2A16}",
        legacy: false,
    },
    NamedRef {
        name: "quest",
        value: "\u{3F}",
        legacy: false,
    },
    NamedRef {
        name: "questeq",
        value: "\u{225F}",
        legacy: false,
    },
    NamedRef {
        name: "quot",
        value: "\u{22}",
        legacy: true,
    },
    NamedRef {
        name: "rAarr",
        value: "\u{21DB}",
        legacy: false,
    },
    NamedRef {
        name: "rArr",
        value: "\u{21D2}",
        legacy: false,
    },
    NamedRef {
        name: "rAtail",
        value: "\u{291C}",
        legacy: false,
    },
    NamedRef {
        name: "rBarr",
        value: "\u{290F}",
        legacy: false,
    },
    NamedRef {
        name: "rHar",
        value: "\u{2964}",
        legacy: false,
    },
    NamedRef {
        name: "race",
        value: "\u{223D}\u{331}",
        legacy: false,
    },
    NamedRef {
        name: "racute",
        value: "\u{155}",
        legacy: false,
    },
    NamedRef {
        name: "radic",
        value: "\u{221A}",
        legacy: false,
    },
    NamedRef {
        name: "raemptyv",
        value: "\u{29B3}",
        legacy: false,
    },
    NamedRef {
        name: "rang",
        value: "\u{27E9}",
        legacy: false,
    },
    NamedRef {
        name: "rangd",
        value: "\u{2992}",
        legacy: false,
    },
    NamedRef {
        name: "range",
        value: "\u{29A5}",
        legacy: false,
    },
    NamedRef {
        name: "rangle",
        value: "\u{27E9}",
        legacy: false,
    },
    NamedRef {
        name: "raquo",
        value: "\u{BB}",
        legacy: true,
    },
    NamedRef {
        name: "rarr",
        value: "\u{2192}",
        legacy: false,
    },
    NamedRef {
        name: "rarrap",
        value: "\u{2975}",
        legacy: false,
    },
    NamedRef {
        name: "rarrb",
        value: "\u{21E5}",
        legacy: false,
    },
    NamedRef {
        name: "rarrbfs",
        value: "\u{2920}",
        legacy: false,
    },
    NamedRef {
        name: "rarrc",
        value: "\u{2933}",
        legacy: false,
    },
    NamedRef {
        name: "rarrfs",
        value: "\u{291E}",
        legacy: false,
    },
    NamedRef {
        name: "rarrhk",
        value: "\u{21AA}",
        legacy: false,
    },
    NamedRef {
        name: "rarrlp",
        value: "\u{21AC}",
        legacy: false,
    },
    NamedRef {
        name: "rarrpl",
        value: "\u{2945}",
        legacy: false,
    },
    NamedRef {
        name: "rarrsim",
        value: "\u{2974}",
        legacy: false,
    },
    NamedRef {
        name: "rarrtl",
        value: "\u{21A3}",
        legacy: false,
    },
    NamedRef {
        name: "rarrw",
        value: "\u{219D}",
        legacy: false,
    },
    NamedRef {
        name: "ratail",
        value: "\u{291A}",
        legacy: false,
    },
    NamedRef {
        name: "ratio",
        value: "\u{2236}",
        legacy: false,
    },
    NamedRef {
        name: "rationals",
        value: "\u{211A}",
        legacy: false,
    },
    NamedRef {
        name: "rbarr",
        value: "\u{290D}",
        legacy: false,
    },
    NamedRef {
        name: "rbbrk",
        value: "\u{2773}",
        legacy: false,
    },
    NamedRef {
        name: "rbrace",
        value: "\u{7D}",
        legacy: false,
    },
    NamedRef {
        name: "rbrack",
        value: "\u{5D}",
        legacy: false,
    },
    NamedRef {
        name: "rbrke",
        value: "\u{298C}",
        legacy: false,
    },
    NamedRef {
        name: "rbrksld",
        value: "\u{298E}",
        legacy: false,
    },
    NamedRef {
        name: "rbrkslu",
        value: "\u{2990}",
        legacy: false,
    },
    NamedRef {
        name: "rcaron",
        value: "\u{159}",
        legacy: false,
    },
    NamedRef {
        name: "rcedil",
        value: "\u{157}",
        legacy: false,
    },
    NamedRef {
        name: "rceil",
        value: "\u{2309}",
        legacy: false,
    },
    NamedRef {
        name: "rcub",
        value: "\u{7D}",
        legacy: false,
    },
    NamedRef {
        name: "rcy",
        value: "\u{440}",
        legacy: false,
    },
    NamedRef {
        name: "rdca",
        value: "\u{2937}",
        legacy: false,
    },
    NamedRef {
        name: "rdldhar",
        value: "\u{2969}",
        legacy: false,
    },
    NamedRef {
        name: "rdquo",
        value: "\u{201D}",
        legacy: false,
    },
    NamedRef {
        name: "rdquor",
        value: "\u{201D}",
        legacy: false,
    },
    NamedRef {
        name: "rdsh",
        value: "\u{21B3}",
        legacy: false,
    },
    NamedRef {
        name: "real",
        value: "\u{211C}",
        legacy: false,
    },
    NamedRef {
        name: "realine",
        value: "\u{211B}",
        legacy: false,
    },
    NamedRef {
        name: "realpart",
        value: "\u{211C}",
        legacy: false,
    },
    NamedRef {
        name: "reals",
        value: "\u{211D}",
        legacy: false,
    },
    NamedRef {
        name: "rect",
        value: "\u{25AD}",
        legacy: false,
    },
    NamedRef {
        name: "reg",
        value: "\u{AE}",
        legacy: true,
    },
    NamedRef {
        name: "rfisht",
        value: "\u{297D}",
        legacy: false,
    },
    NamedRef {
        name: "rfloor",
        value: "\u{230B}",
        legacy: false,
    },
    NamedRef {
        name: "rfr",
        value: "\u{1D52F}",
        legacy: false,
    },
    NamedRef {
        name: "rhard",
        value: "\u{21C1}",
        legacy: false,
    },
    NamedRef {
        name: "rharu",
        value: "\u{21C0}",
        legacy: false,
    },
    NamedRef {
        name: "rharul",
        value: "\u{296C}",
        legacy: false,
    },
    NamedRef {
        name: "rho",
        value: "\u{3C1}",
        legacy: false,
    },
    NamedRef {
        name: "rhov",
        value: "\u{3F1}",
        legacy: false,
    },
    NamedRef {
        name: "rightarrow",
        value: "\u{2192}",
        legacy: false,
    },
    NamedRef {
        name: "rightarrowtail",
        value: "\u{21A3}",
        legacy: false,
    },
    NamedRef {
        name: "rightharpoondown",
        value: "\u{21C1}",
        legacy: false,
    },
    NamedRef {
        name: "rightharpoonup",
        value: "\u{21C0}",
        legacy: false,
    },
    NamedRef {
        name: "rightleftarrows",
        value: "\u{21C4}",
        legacy: false,
    },
    NamedRef {
        name: "rightleftharpoons",
        value: "\u{21CC}",
        legacy: false,
    },
    NamedRef {
        name: "rightrightarrows",
        value: "\u{21C9}",
        legacy: false,
    },
    NamedRef {
        name: "rightsquigarrow",
        value: "\u{219D}",
        legacy: false,
    },
    NamedRef {
        name: "rightthreetimes",
        value: "\u{22CC}",
        legacy: false,
    },
    NamedRef {
        name: "ring",
        value: "\u{2DA}",
        legacy: false,
    },
    NamedRef {
        name: "risingdotseq",
        value: "\u{2253}",
        legacy: false,
    },
    NamedRef {
        name: "rlarr",
        value: "\u{21C4}",
        legacy: false,
    },
    NamedRef {
        name: "rlhar",
        value: "\u{21CC}",
        legacy: false,
    },
    NamedRef {
        name: "rlm",
        value: "\u{200F}",
        legacy: false,
    },
    NamedRef {
        name: "rmoust",
        value: "\u{23B1}",
        legacy: false,
    },
    NamedRef {
        name: "rmoustache",
        value: "\u{23B1}",
        legacy: false,
    },
    NamedRef {
        name: "rnmid",
        value: "\u{2AEE}",
        legacy: false,
    },
    NamedRef {
        name: "roang",
        value: "\u{27ED}",
        legacy: false,
    },
    NamedRef {
        name: "roarr",
        value: "\u{21FE}",
        legacy: false,
    },
    NamedRef {
        name: "robrk",
        value: "\u{27E7}",
        legacy: false,
    },
    NamedRef {
        name: "ropar",
        value: "\u{2986}",
        legacy: false,
    },
    NamedRef {
        name: "ropf",
        value: "\u{1D563}",
        legacy: false,
    },
    NamedRef {
        name: "roplus",
        value: "\u{2A2E}",
        legacy: false,
    },
    NamedRef {
        name: "rotimes",
        value: "\u{2A35}",
        legacy: false,
    },
    NamedRef {
        name: "rpar",
        value: "\u{29}",
        legacy: false,
    },
    NamedRef {
        name: "rpargt",
        value: "\u{2994}",
        legacy: false,
    },
    NamedRef {
        name: "rppolint",
        value: "\u{2A12}",
        legacy: false,
    },
    NamedRef {
        name: "rrarr",
        value: "\u{21C9}",
        legacy: false,
    },
    NamedRef {
        name: "rsaquo",
        value: "\u{203A}",
        legacy: false,
    },
    NamedRef {
        name: "rscr",
        value: "\u{1D4C7}",
        legacy: false,
    },
    NamedRef {
        name: "rsh",
        value: "\u{21B1}",
        legacy: false,
    },
    NamedRef {
        name: "rsqb",
        value: "\u{5D}",
        legacy: false,
    },
    NamedRef {
        name: "rsquo",
        value: "\u{2019}",
        legacy: false,
    },
    NamedRef {
        name: "rsquor",
        value: "\u{2019}",
        legacy: false,
    },
    NamedRef {
        name: "rthree",
        value: "\u{22CC}",
        legacy: false,
    },
    NamedRef {
        name: "rtimes",
        value: "\u{22CA}",
        legacy: false,
    },
    NamedRef {
        name: "rtri",
        value: "\u{25B9}",
        legacy: false,
    },
    NamedRef {
        name: "rtrie",
        value: "\u{22B5}",
        legacy: false,
    },
    NamedRef {
        name: "rtrif",
        value: "\u{25B8}",
        legacy: false,
    },
    NamedRef {
        name: "rtriltri",
        value: "\u{29CE}",
        legacy: false,
    },
    NamedRef {
        name: "ruluhar",
        value: "\u{2968}",
        legacy: false,
    },
    NamedRef {
        name: "rx",
        value: "\u{211E}",
        legacy: false,
    },
    NamedRef {
        name: "sacute",
        value: "\u{15B}",
        legacy: false,
    },
    NamedRef {
        name: "sbquo",
        value: "\u{201A}",
        legacy: false,
    },
    NamedRef {
        name: "sc",
        value: "\u{227B}",
        legacy: false,
    },
    NamedRef {
        name: "scE",
        value: "\u{2AB4}",
        legacy: false,
    },
    NamedRef {
        name: "scap",
        value: "\u{2AB8}",
        legacy: false,
    },
    NamedRef {
        name: "scaron",
        value: "\u{161}",
        legacy: false,
    },
    NamedRef {
        name: "sccue",
        value: "\u{227D}",
        legacy: false,
    },
    NamedRef {
        name: "sce",
        value: "\u{2AB0}",
        legacy: false,
    },
    NamedRef {
        name: "scedil",
        value: "\u{15F}",
        legacy: false,
    },
    NamedRef {
        name: "scirc",
        value: "\u{15D}",
        legacy: false,
    },
    NamedRef {
        name: "scnE",
        value: "\u{2AB6}",
        legacy: false,
    },
    NamedRef {
        name: "scnap",
        value: "\u{2ABA}",
        legacy: false,
    },
    NamedRef {
        name: "scnsim",
        value: "\u{22E9}",
        legacy: false,
    },
    NamedRef {
        name: "scpolint",
        value: "\u{2A13}",
        legacy: false,
    },
    NamedRef {
        name: "scsim",
        value: "\u{227F}",
        legacy: false,
    },
    NamedRef {
        name: "scy",
        value: "\u{441}",
        legacy: false,
    },
    NamedRef {
        name: "sdot",
        value: "\u{22C5}",
        legacy: false,
    },
    NamedRef {
        name: "sdotb",
        value: "\u{22A1}",
        legacy: false,
    },
    NamedRef {
        name: "sdote",
        value: "\u{2A66}",
        legacy: false,
    },
    NamedRef {
        name: "seArr",
        value: "\u{21D8}",
        legacy: false,
    },
    NamedRef {
        name: "searhk",
        value: "\u{2925}",
        legacy: false,
    },
    NamedRef {
        name: "searr",
        value: "\u{2198}",
        legacy: false,
    },
    NamedRef {
        name: "searrow",
        value: "\u{2198}",
        legacy: false,
    },
    NamedRef {
        name: "sect",
        value: "\u{A7}",
        legacy: true,
    },
    NamedRef {
        name: "semi",
        value: "\u{3B}",
        legacy: false,
    },
    NamedRef {
        name: "seswar",
        value: "\u{2929}",
        legacy: false,
    },
    NamedRef {
        name: "setminus",
        value: "\u{2216}",
        legacy: false,
    },
    NamedRef {
        name: "setmn",
        value: "\u{2216}",
        legacy: false,
    },
    NamedRef {
        name: "sext",
        value: "\u{2736}",
        legacy: false,
    },
    NamedRef {
        name: "sfr",
        value: "\u{1D530}",
        legacy: false,
    },
    NamedRef {
        name: "sfrown",
        value: "\u{2322}",
        legacy: false,
    },
    NamedRef {
        name: "sharp",
        value: "\u{266F}",
        legacy: false,
    },
    NamedRef {
        name: "shchcy",
        value: "\u{449}",
        legacy: false,
    },
    NamedRef {
        name: "shcy",
        value: "\u{448}",
        legacy: false,
    },
    NamedRef {
        name: "shortmid",
        value: "\u{2223}",
        legacy: false,
    },
    NamedRef {
        name: "shortparallel",
        value: "\u{2225}",
        legacy: false,
    },
    NamedRef {
        name: "shy",
        value: "\u{AD}",
        legacy: true,
    },
    NamedRef {
        name: "sigma",
        value: "\u{3C3}",
        legacy: false,
    },
    NamedRef {
        name: "sigmaf",
        value: "\u{3C2}",
        legacy: false,
    },
    NamedRef {
        name: "sigmav",
        value: "\u{3C2}",
        legacy: false,
    },
    NamedRef {
        name: "sim",
        value: "\u{223C}",
        legacy: false,
    },
    NamedRef {
        name: "simdot",
        value: "\u{2A6A}",
        legacy: false,
    },
    NamedRef {
        name: "sime",
        value: "\u{2243}",
        legacy: false,
    },
    NamedRef {
        name: "simeq",
        value: "\u{2243}",
        legacy: false,
    },
    NamedRef {
        name: "simg",
        value: "\u{2A9E}",
        legacy: false,
    },
    NamedRef {
        name: "simgE",
        value: "\u{2AA0}",
        legacy: false,
    },
    NamedRef {
        name: "siml",
        value: "\u{2A9D}",
        legacy: false,
    },
    NamedRef {
        name: "simlE",
        value: "\u{2A9F}",
        legacy: false,
    },
    NamedRef {
        name: "simne",
        value: "\u{2246}",
        legacy: false,
    },
    NamedRef {
        name: "simplus",
        value: "\u{2A24}",
        legacy: false,
    },
    NamedRef {
        name: "simrarr",
        value: "\u{2972}",
        legacy: false,
    },
    NamedRef {
        name: "slarr",
        value: "\u{2190}",
        legacy: false,
    },
    NamedRef {
        name: "smallsetminus",
        value: "\u{2216}",
        legacy: false,
    },
    NamedRef {
        name: "smashp",
        value: "\u{2A33}",
        legacy: false,
    },
    NamedRef {
        name: "smeparsl",
        value: "\u{29E4}",
        legacy: false,
    },
    NamedRef {
        name: "smid",
        value: "\u{2223}",
        legacy: false,
    },
    NamedRef {
        name: "smile",
        value: "\u{2323}",
        legacy: false,
    },
    NamedRef {
        name: "smt",
        value: "\u{2AAA}",
        legacy: false,
    },
    NamedRef {
        name: "smte",
        value: "\u{2AAC}",
        legacy: false,
    },
    NamedRef {
        name: "smtes",
        value: "\u{2AAC}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "softcy",
        value: "\u{44C}",
        legacy: false,
    },
    NamedRef {
        name: "sol",
        value: "\u{2F}",
        legacy: false,
    },
    NamedRef {
        name: "solb",
        value: "\u{29C4}",
        legacy: false,
    },
    NamedRef {
        name: "solbar",
        value: "\u{233F}",
        legacy: false,
    },
    NamedRef {
        name: "sopf",
        value: "\u{1D564}",
        legacy: false,
    },
    NamedRef {
        name: "spades",
        value: "\u{2660}",
        legacy: false,
    },
    NamedRef {
        name: "spadesuit",
        value: "\u{2660}",
        legacy: false,
    },
    NamedRef {
        name: "spar",
        value: "\u{2225}",
        legacy: false,
    },
    NamedRef {
        name: "sqcap",
        value: "\u{2293}",
        legacy: false,
    },
    NamedRef {
        name: "sqcaps",
        value: "\u{2293}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "sqcup",
        value: "\u{2294}",
        legacy: false,
    },
    NamedRef {
        name: "sqcups",
        value: "\u{2294}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "sqsub",
        value: "\u{228F}",
        legacy: false,
    },
    NamedRef {
        name: "sqsube",
        value: "\u{2291}",
        legacy: false,
    },
    NamedRef {
        name: "sqsubset",
        value: "\u{228F}",
        legacy: false,
    },
    NamedRef {
        name: "sqsubseteq",
        value: "\u{2291}",
        legacy: false,
    },
    NamedRef {
        name: "sqsup",
        value: "\u{2290}",
        legacy: false,
    },
    NamedRef {
        name: "sqsupe",
        value: "\u{2292}",
        legacy: false,
    },
    NamedRef {
        name: "sqsupset",
        value: "\u{2290}",
        legacy: false,
    },
    NamedRef {
        name: "sqsupseteq",
        value: "\u{2292}",
        legacy: false,
    },
    NamedRef {
        name: "squ",
        value: "\u{25A1}",
        legacy: false,
    },
    NamedRef {
        name: "square",
        value: "\u{25A1}",
        legacy: false,
    },
    NamedRef {
        name: "squarf",
        value: "\u{25AA}",
        legacy: false,
    },
    NamedRef {
        name: "squf",
        value: "\u{25AA}",
        legacy: false,
    },
    NamedRef {
        name: "srarr",
        value: "\u{2192}",
        legacy: false,
    },
    NamedRef {
        name: "sscr",
        value: "\u{1D4C8}",
        legacy: false,
    },
    NamedRef {
        name: "ssetmn",
        value: "\u{2216}",
        legacy: false,
    },
    NamedRef {
        name: "ssmile",
        value: "\u{2323}",
        legacy: false,
    },
    NamedRef {
        name: "sstarf",
        value: "\u{22C6}",
        legacy: false,
    },
    NamedRef {
        name: "star",
        value: "\u{2606}",
        legacy: false,
    },
    NamedRef {
        name: "starf",
        value: "\u{2605}",
        legacy: false,
    },
    NamedRef {
        name: "straightepsilon",
        value: "\u{3F5}",
        legacy: false,
    },
    NamedRef {
        name: "straightphi",
        value: "\u{3D5}",
        legacy: false,
    },
    NamedRef {
        name: "strns",
        value: "\u{AF}",
        legacy: false,
    },
    NamedRef {
        name: "sub",
        value: "\u{2282}",
        legacy: false,
    },
    NamedRef {
        name: "subE",
        value: "\u{2AC5}",
        legacy: false,
    },
    NamedRef {
        name: "subdot",
        value: "\u{2ABD}",
        legacy: false,
    },
    NamedRef {
        name: "sube",
        value: "\u{2286}",
        legacy: false,
    },
    NamedRef {
        name: "subedot",
        value: "\u{2AC3}",
        legacy: false,
    },
    NamedRef {
        name: "submult",
        value: "\u{2AC1}",
        legacy: false,
    },
    NamedRef {
        name: "subnE",
        value: "\u{2ACB}",
        legacy: false,
    },
    NamedRef {
        name: "subne",
        value: "\u{228A}",
        legacy: false,
    },
    NamedRef {
        name: "subplus",
        value: "\u{2ABF}",
        legacy: false,
    },
    NamedRef {
        name: "subrarr",
        value: "\u{2979}",
        legacy: false,
    },
    NamedRef {
        name: "subset",
        value: "\u{2282}",
        legacy: false,
    },
    NamedRef {
        name: "subseteq",
        value: "\u{2286}",
        legacy: false,
    },
    NamedRef {
        name: "subseteqq",
        value: "\u{2AC5}",
        legacy: false,
    },
    NamedRef {
        name: "subsetneq",
        value: "\u{228A}",
        legacy: false,
    },
    NamedRef {
        name: "subsetneqq",
        value: "\u{2ACB}",
        legacy: false,
    },
    NamedRef {
        name: "subsim",
        value: "\u{2AC7}",
        legacy: false,
    },
    NamedRef {
        name: "subsub",
        value: "\u{2AD5}",
        legacy: false,
    },
    NamedRef {
        name: "subsup",
        value: "\u{2AD3}",
        legacy: false,
    },
    NamedRef {
        name: "succ",
        value: "\u{227B}",
        legacy: false,
    },
    NamedRef {
        name: "succapprox",
        value: "\u{2AB8}",
        legacy: false,
    },
    NamedRef {
        name: "succcurlyeq",
        value: "\u{227D}",
        legacy: false,
    },
    NamedRef {
        name: "succeq",
        value: "\u{2AB0}",
        legacy: false,
    },
    NamedRef {
        name: "succnapprox",
        value: "\u{2ABA}",
        legacy: false,
    },
    NamedRef {
        name: "succneqq",
        value: "\u{2AB6}",
        legacy: false,
    },
    NamedRef {
        name: "succnsim",
        value: "\u{22E9}",
        legacy: false,
    },
    NamedRef {
        name: "succsim",
        value: "\u{227F}",
        legacy: false,
    },
    NamedRef {
        name: "sum",
        value: "\u{2211}",
        legacy: false,
    },
    NamedRef {
        name: "sung",
        value: "\u{266A}",
        legacy: false,
    },
    NamedRef {
        name: "sup",
        value: "\u{2283}",
        legacy: false,
    },
    NamedRef {
        name: "sup1",
        value: "\u{B9}",
        legacy: true,
    },
    NamedRef {
        name: "sup2",
        value: "\u{B2}",
        legacy: true,
    },
    NamedRef {
        name: "sup3",
        value: "\u{B3}",
        legacy: true,
    },
    NamedRef {
        name: "supE",
        value: "\u{2AC6}",
        legacy: false,
    },
    NamedRef {
        name: "supdot",
        value: "\u{2ABE}",
        legacy: false,
    },
    NamedRef {
        name: "supdsub",
        value: "\u{2AD8}",
        legacy: false,
    },
    NamedRef {
        name: "supe",
        value: "\u{2287}",
        legacy: false,
    },
    NamedRef {
        name: "supedot",
        value: "\u{2AC4}",
        legacy: false,
    },
    NamedRef {
        name: "suphsol",
        value: "\u{27C9}",
        legacy: false,
    },
    NamedRef {
        name: "suphsub",
        value: "\u{2AD7}",
        legacy: false,
    },
    NamedRef {
        name: "suplarr",
        value: "\u{297B}",
        legacy: false,
    },
    NamedRef {
        name: "supmult",
        value: "\u{2AC2}",
        legacy: false,
    },
    NamedRef {
        name: "supnE",
        value: "\u{2ACC}",
        legacy: false,
    },
    NamedRef {
        name: "supne",
        value: "\u{228B}",
        legacy: false,
    },
    NamedRef {
        name: "supplus",
        value: "\u{2AC0}",
        legacy: false,
    },
    NamedRef {
        name: "supset",
        value: "\u{2283}",
        legacy: false,
    },
    NamedRef {
        name: "supseteq",
        value: "\u{2287}",
        legacy: false,
    },
    NamedRef {
        name: "supseteqq",
        value: "\u{2AC6}",
        legacy: false,
    },
    NamedRef {
        name: "supsetneq",
        value: "\u{228B}",
        legacy: false,
    },
    NamedRef {
        name: "supsetneqq",
        value: "\u{2ACC}",
        legacy: false,
    },
    NamedRef {
        name: "supsim",
        value: "\u{2AC8}",
        legacy: false,
    },
    NamedRef {
        name: "supsub",
        value: "\u{2AD4}",
        legacy: false,
    },
    NamedRef {
        name: "supsup",
        value: "\u{2AD6}",
        legacy: false,
    },
    NamedRef {
        name: "swArr",
        value: "\u{21D9}",
        legacy: false,
    },
    NamedRef {
        name: "swarhk",
        value: "\u{2926}",
        legacy: false,
    },
    NamedRef {
        name: "swarr",
        value: "\u{2199}",
        legacy: false,
    },
    NamedRef {
        name: "swarrow",
        value: "\u{2199}",
        legacy: false,
    },
    NamedRef {
        name: "swnwar",
        value: "\u{292A}",
        legacy: false,
    },
    NamedRef {
        name: "szlig",
        value: "\u{DF}",
        legacy: true,
    },
    NamedRef {
        name: "target",
        value: "\u{2316}",
        legacy: false,
    },
    NamedRef {
        name: "tau",
        value: "\u{3C4}",
        legacy: false,
    },
    NamedRef {
        name: "tbrk",
        value: "\u{23B4}",
        legacy: false,
    },
    NamedRef {
        name: "tcaron",
        value: "\u{165}",
        legacy: false,
    },
    NamedRef {
        name: "tcedil",
        value: "\u{163}",
        legacy: false,
    },
    NamedRef {
        name: "tcy",
        value: "\u{442}",
        legacy: false,
    },
    NamedRef {
        name: "tdot",
        value: "\u{20DB}",
        legacy: false,
    },
    NamedRef {
        name: "telrec",
        value: "\u{2315}",
        legacy: false,
    },
    NamedRef {
        name: "tfr",
        value: "\u{1D531}",
        legacy: false,
    },
    NamedRef {
        name: "there4",
        value: "\u{2234}",
        legacy: false,
    },
    NamedRef {
        name: "therefore",
        value: "\u{2234}",
        legacy: false,
    },
    NamedRef {
        name: "theta",
        value: "\u{3B8}",
        legacy: false,
    },
    NamedRef {
        name: "thetasym",
        value: "\u{3D1}",
        legacy: false,
    },
    NamedRef {
        name: "thetav",
        value: "\u{3D1}",
        legacy: false,
    },
    NamedRef {
        name: "thickapprox",
        value: "\u{2248}",
        legacy: false,
    },
    NamedRef {
        name: "thicksim",
        value: "\u{223C}",
        legacy: false,
    },
    NamedRef {
        name: "thinsp",
        value: "\u{2009}",
        legacy: false,
    },
    NamedRef {
        name: "thkap",
        value: "\u{2248}",
        legacy: false,
    },
    NamedRef {
        name: "thksim",
        value: "\u{223C}",
        legacy: false,
    },
    NamedRef {
        name: "thorn",
        value: "\u{FE}",
        legacy: true,
    },
    NamedRef {
        name: "tilde",
        value: "\u{2DC}",
        legacy: false,
    },
    NamedRef {
        name: "times",
        value: "\u{D7}",
        legacy: true,
    },
    NamedRef {
        name: "timesb",
        value: "\u{22A0}",
        legacy: false,
    },
    NamedRef {
        name: "timesbar",
        value: "\u{2A31}",
        legacy: false,
    },
    NamedRef {
        name: "timesd",
        value: "\u{2A30}",
        legacy: false,
    },
    NamedRef {
        name: "tint",
        value: "\u{222D}",
        legacy: false,
    },
    NamedRef {
        name: "toea",
        value: "\u{2928}",
        legacy: false,
    },
    NamedRef {
        name: "top",
        value: "\u{22A4}",
        legacy: false,
    },
    NamedRef {
        name: "topbot",
        value: "\u{2336}",
        legacy: false,
    },
    NamedRef {
        name: "topcir",
        value: "\u{2AF1}",
        legacy: false,
    },
    NamedRef {
        name: "topf",
        value: "\u{1D565}",
        legacy: false,
    },
    NamedRef {
        name: "topfork",
        value: "\u{2ADA}",
        legacy: false,
    },
    NamedRef {
        name: "tosa",
        value: "\u{2929}",
        legacy: false,
    },
    NamedRef {
        name: "tprime",
        value: "\u{2034}",
        legacy: false,
    },
    NamedRef {
        name: "trade",
        value: "\u{2122}",
        legacy: false,
    },
    NamedRef {
        name: "triangle",
        value: "\u{25B5}",
        legacy: false,
    },
    NamedRef {
        name: "triangledown",
        value: "\u{25BF}",
        legacy: false,
    },
    NamedRef {
        name: "triangleleft",
        value: "\u{25C3}",
        legacy: false,
    },
    NamedRef {
        name: "trianglelefteq",
        value: "\u{22B4}",
        legacy: false,
    },
    NamedRef {
        name: "triangleq",
        value: "\u{225C}",
        legacy: false,
    },
    NamedRef {
        name: "triangleright",
        value: "\u{25B9}",
        legacy: false,
    },
    NamedRef {
        name: "trianglerighteq",
        value: "\u{22B5}",
        legacy: false,
    },
    NamedRef {
        name: "tridot",
        value: "\u{25EC}",
        legacy: false,
    },
    NamedRef {
        name: "trie",
        value: "\u{225C}",
        legacy: false,
    },
    NamedRef {
        name: "triminus",
        value: "\u{2A3A}",
        legacy: false,
    },
    NamedRef {
        name: "triplus",
        value: "\u{2A39}",
        legacy: false,
    },
    NamedRef {
        name: "trisb",
        value: "\u{29CD}",
        legacy: false,
    },
    NamedRef {
        name: "tritime",
        value: "\u{2A3B}",
        legacy: false,
    },
    NamedRef {
        name: "trpezium",
        value: "\u{23E2}",
        legacy: false,
    },
    NamedRef {
        name: "tscr",
        value: "\u{1D4C9}",
        legacy: false,
    },
    NamedRef {
        name: "tscy",
        value: "\u{446}",
        legacy: false,
    },
    NamedRef {
        name: "tshcy",
        value: "\u{45B}",
        legacy: false,
    },
    NamedRef {
        name: "tstrok",
        value: "\u{167}",
        legacy: false,
    },
    NamedRef {
        name: "twixt",
        value: "\u{226C}",
        legacy: false,
    },
    NamedRef {
        name: "twoheadleftarrow",
        value: "\u{219E}",
        legacy: false,
    },
    NamedRef {
        name: "twoheadrightarrow",
        value: "\u{21A0}",
        legacy: false,
    },
    NamedRef {
        name: "uArr",
        value: "\u{21D1}",
        legacy: false,
    },
    NamedRef {
        name: "uHar",
        value: "\u{2963}",
        legacy: false,
    },
    NamedRef {
        name: "uacute",
        value: "\u{FA}",
        legacy: true,
    },
    NamedRef {
        name: "uarr",
        value: "\u{2191}",
        legacy: false,
    },
    NamedRef {
        name: "ubrcy",
        value: "\u{45E}",
        legacy: false,
    },
    NamedRef {
        name: "ubreve",
        value: "\u{16D}",
        legacy: false,
    },
    NamedRef {
        name: "ucirc",
        value: "\u{FB}",
        legacy: true,
    },
    NamedRef {
        name: "ucy",
        value: "\u{443}",
        legacy: false,
    },
    NamedRef {
        name: "udarr",
        value: "\u{21C5}",
        legacy: false,
    },
    NamedRef {
        name: "udblac",
        value: "\u{171}",
        legacy: false,
    },
    NamedRef {
        name: "udhar",
        value: "\u{296E}",
        legacy: false,
    },
    NamedRef {
        name: "ufisht",
        value: "\u{297E}",
        legacy: false,
    },
    NamedRef {
        name: "ufr",
        value: "\u{1D532}",
        legacy: false,
    },
    NamedRef {
        name: "ugrave",
        value: "\u{F9}",
        legacy: true,
    },
    NamedRef {
        name: "uharl",
        value: "\u{21BF}",
        legacy: false,
    },
    NamedRef {
        name: "uharr",
        value: "\u{21BE}",
        legacy: false,
    },
    NamedRef {
        name: "uhblk",
        value: "\u{2580}",
        legacy: false,
    },
    NamedRef {
        name: "ulcorn",
        value: "\u{231C}",
        legacy: false,
    },
    NamedRef {
        name: "ulcorner",
        value: "\u{231C}",
        legacy: false,
    },
    NamedRef {
        name: "ulcrop",
        value: "\u{230F}",
        legacy: false,
    },
    NamedRef {
        name: "ultri",
        value: "\u{25F8}",
        legacy: false,
    },
    NamedRef {
        name: "umacr",
        value: "\u{16B}",
        legacy: false,
    },
    NamedRef {
        name: "uml",
        value: "\u{A8}",
        legacy: true,
    },
    NamedRef {
        name: "uogon",
        value: "\u{173}",
        legacy: false,
    },
    NamedRef {
        name: "uopf",
        value: "\u{1D566}",
        legacy: false,
    },
    NamedRef {
        name: "uparrow",
        value: "\u{2191}",
        legacy: false,
    },
    NamedRef {
        name: "updownarrow",
        value: "\u{2195}",
        legacy: false,
    },
    NamedRef {
        name: "upharpoonleft",
        value: "\u{21BF}",
        legacy: false,
    },
    NamedRef {
        name: "upharpoonright",
        value: "\u{21BE}",
        legacy: false,
    },
    NamedRef {
        name: "uplus",
        value: "\u{228E}",
        legacy: false,
    },
    NamedRef {
        name: "upsi",
        value: "\u{3C5}",
        legacy: false,
    },
    NamedRef {
        name: "upsih",
        value: "\u{3D2}",
        legacy: false,
    },
    NamedRef {
        name: "upsilon",
        value: "\u{3C5}",
        legacy: false,
    },
    NamedRef {
        name: "upuparrows",
        value: "\u{21C8}",
        legacy: false,
    },
    NamedRef {
        name: "urcorn",
        value: "\u{231D}",
        legacy: false,
    },
    NamedRef {
        name: "urcorner",
        value: "\u{231D}",
        legacy: false,
    },
    NamedRef {
        name: "urcrop",
        value: "\u{230E}",
        legacy: false,
    },
    NamedRef {
        name: "uring",
        value: "\u{16F}",
        legacy: false,
    },
    NamedRef {
        name: "urtri",
        value: "\u{25F9}",
        legacy: false,
    },
    NamedRef {
        name: "uscr",
        value: "\u{1D4CA}",
        legacy: false,
    },
    NamedRef {
        name: "utdot",
        value: "\u{22F0}",
        legacy: false,
    },
    NamedRef {
        name: "utilde",
        value: "\u{169}",
        legacy: false,
    },
    NamedRef {
        name: "utri",
        value: "\u{25B5}",
        legacy: false,
    },
    NamedRef {
        name: "utrif",
        value: "\u{25B4}",
        legacy: false,
    },
    NamedRef {
        name: "uuarr",
        value: "\u{21C8}",
        legacy: false,
    },
    NamedRef {
        name: "uuml",
        value: "\u{FC}",
        legacy: true,
    },
    NamedRef {
        name: "uwangle",
        value: "\u{29A7}",
        legacy: false,
    },
    NamedRef {
        name: "vArr",
        value: "\u{21D5}",
        legacy: false,
    },
    NamedRef {
        name: "vBar",
        value: "\u{2AE8}",
        legacy: false,
    },
    NamedRef {
        name: "vBarv",
        value: "\u{2AE9}",
        legacy: false,
    },
    NamedRef {
        name: "vDash",
        value: "\u{22A8}",
        legacy: false,
    },
    NamedRef {
        name: "vangrt",
        value: "\u{299C}",
        legacy: false,
    },
    NamedRef {
        name: "varepsilon",
        value: "\u{3F5}",
        legacy: false,
    },
    NamedRef {
        name: "varkappa",
        value: "\u{3F0}",
        legacy: false,
    },
    NamedRef {
        name: "varnothing",
        value: "\u{2205}",
        legacy: false,
    },
    NamedRef {
        name: "varphi",
        value: "\u{3D5}",
        legacy: false,
    },
    NamedRef {
        name: "varpi",
        value: "\u{3D6}",
        legacy: false,
    },
    NamedRef {
        name: "varpropto",
        value: "\u{221D}",
        legacy: false,
    },
    NamedRef {
        name: "varr",
        value: "\u{2195}",
        legacy: false,
    },
    NamedRef {
        name: "varrho",
        value: "\u{3F1}",
        legacy: false,
    },
    NamedRef {
        name: "varsigma",
        value: "\u{3C2}",
        legacy: false,
    },
    NamedRef {
        name: "varsubsetneq",
        value: "\u{228A}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "varsubsetneqq",
        value: "\u{2ACB}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "varsupsetneq",
        value: "\u{228B}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "varsupsetneqq",
        value: "\u{2ACC}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "vartheta",
        value: "\u{3D1}",
        legacy: false,
    },
    NamedRef {
        name: "vartriangleleft",
        value: "\u{22B2}",
        legacy: false,
    },
    NamedRef {
        name: "vartriangleright",
        value: "\u{22B3}",
        legacy: false,
    },
    NamedRef {
        name: "vcy",
        value: "\u{432}",
        legacy: false,
    },
    NamedRef {
        name: "vdash",
        value: "\u{22A2}",
        legacy: false,
    },
    NamedRef {
        name: "vee",
        value: "\u{2228}",
        legacy: false,
    },
    NamedRef {
        name: "veebar",
        value: "\u{22BB}",
        legacy: false,
    },
    NamedRef {
        name: "veeeq",
        value: "\u{225A}",
        legacy: false,
    },
    NamedRef {
        name: "vellip",
        value: "\u{22EE}",
        legacy: false,
    },
    NamedRef {
        name: "verbar",
        value: "\u{7C}",
        legacy: false,
    },
    NamedRef {
        name: "vert",
        value: "\u{7C}",
        legacy: false,
    },
    NamedRef {
        name: "vfr",
        value: "\u{1D533}",
        legacy: false,
    },
    NamedRef {
        name: "vltri",
        value: "\u{22B2}",
        legacy: false,
    },
    NamedRef {
        name: "vnsub",
        value: "\u{2282}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "vnsup",
        value: "\u{2283}\u{20D2}",
        legacy: false,
    },
    NamedRef {
        name: "vopf",
        value: "\u{1D567}",
        legacy: false,
    },
    NamedRef {
        name: "vprop",
        value: "\u{221D}",
        legacy: false,
    },
    NamedRef {
        name: "vrtri",
        value: "\u{22B3}",
        legacy: false,
    },
    NamedRef {
        name: "vscr",
        value: "\u{1D4CB}",
        legacy: false,
    },
    NamedRef {
        name: "vsubnE",
        value: "\u{2ACB}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "vsubne",
        value: "\u{228A}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "vsupnE",
        value: "\u{2ACC}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "vsupne",
        value: "\u{228B}\u{FE00}",
        legacy: false,
    },
    NamedRef {
        name: "vzigzag",
        value: "\u{299A}",
        legacy: false,
    },
    NamedRef {
        name: "wcirc",
        value: "\u{175}",
        legacy: false,
    },
    NamedRef {
        name: "wedbar",
        value: "\u{2A5F}",
        legacy: false,
    },
    NamedRef {
        name: "wedge",
        value: "\u{2227}",
        legacy: false,
    },
    NamedRef {
        name: "wedgeq",
        value: "\u{2259}",
        legacy: false,
    },
    NamedRef {
        name: "weierp",
        value: "\u{2118}",
        legacy: false,
    },
    NamedRef {
        name: "wfr",
        value: "\u{1D534}",
        legacy: false,
    },
    NamedRef {
        name: "wopf",
        value: "\u{1D568}",
        legacy: false,
    },
    NamedRef {
        name: "wp",
        value: "\u{2118}",
        legacy: false,
    },
    NamedRef {
        name: "wr",
        value: "\u{2240}",
        legacy: false,
    },
    NamedRef {
        name: "wreath",
        value: "\u{2240}",
        legacy: false,
    },
    NamedRef {
        name: "wscr",
        value: "\u{1D4CC}",
        legacy: false,
    },
    NamedRef {
        name: "xcap",
        value: "\u{22C2}",
        legacy: false,
    },
    NamedRef {
        name: "xcirc",
        value: "\u{25EF}",
        legacy: false,
    },
    NamedRef {
        name: "xcup",
        value: "\u{22C3}",
        legacy: false,
    },
    NamedRef {
        name: "xdtri",
        value: "\u{25BD}",
        legacy: false,
    },
    NamedRef {
        name: "xfr",
        value: "\u{1D535}",
        legacy: false,
    },
    NamedRef {
        name: "xhArr",
        value: "\u{27FA}",
        legacy: false,
    },
    NamedRef {
        name: "xharr",
        value: "\u{27F7}",
        legacy: false,
    },
    NamedRef {
        name: "xi",
        value: "\u{3BE}",
        legacy: false,
    },
    NamedRef {
        name: "xlArr",
        value: "\u{27F8}",
        legacy: false,
    },
    NamedRef {
        name: "xlarr",
        value: "\u{27F5}",
        legacy: false,
    },
    NamedRef {
        name: "xmap",
        value: "\u{27FC}",
        legacy: false,
    },
    NamedRef {
        name: "xnis",
        value: "\u{22FB}",
        legacy: false,
    },
    NamedRef {
        name: "xodot",
        value: "\u{2A00}",
        legacy: false,
    },
    NamedRef {
        name: "xopf",
        value: "\u{1D569}",
        legacy: false,
    },
    NamedRef {
        name: "xoplus",
        value: "\u{2A01}",
        legacy: false,
    },
    NamedRef {
        name: "xotime",
        value: "\u{2A02}",
        legacy: false,
    },
    NamedRef {
        name: "xrArr",
        value: "\u{27F9}",
        legacy: false,
    },
    NamedRef {
        name: "xrarr",
        value: "\u{27F6}",
        legacy: false,
    },
    NamedRef {
        name: "xscr",
        value: "\u{1D4CD}",
        legacy: false,
    },
    NamedRef {
        name: "xsqcup",
        value: "\u{2A06}",
        legacy: false,
    },
    NamedRef {
        name: "xuplus",
        value: "\u{2A04}",
        legacy: false,
    },
    NamedRef {
        name: "xutri",
        value: "\u{25B3}",
        legacy: false,
    },
    NamedRef {
        name: "xvee",
        value: "\u{22C1}",
        legacy: false,
    },
    NamedRef {
        name: "xwedge",
        value: "\u{22C0}",
        legacy: false,
    },
    NamedRef {
        name: "yacute",
        value: "\u{FD}",
        legacy: true,
    },
    NamedRef {
        name: "yacy",
        value: "\u{44F}",
        legacy: false,
    },
    NamedRef {
        name: "ycirc",
        value: "\u{177}",
        legacy: false,
    },
    NamedRef {
        name: "ycy",
        value: "\u{44B}",
        legacy: false,
    },
    NamedRef {
        name: "yen",
        value: "\u{A5}",
        legacy: true,
    },
    NamedRef {
        name: "yfr",
        value: "\u{1D536}",
        legacy: false,
    },
    NamedRef {
        name: "yicy",
        value: "\u{457}",
        legacy: false,
    },
    NamedRef {
        name: "yopf",
        value: "\u{1D56A}",
        legacy: false,
    },
    NamedRef {
        name: "yscr",
        value: "\u{1D4CE}",
        legacy: false,
    },
    NamedRef {
        name: "yucy",
        value: "\u{44E}",
        legacy: false,
    },
    NamedRef {
        name: "yuml",
        value: "\u{FF}",
        legacy: true,
    },
    NamedRef {
        name: "zacute",
        value: "\u{17A}",
        legacy: false,
    },
    NamedRef {
        name: "zcaron",
        value: "\u{17E}",
        legacy: false,
    },
    NamedRef {
        name: "zcy",
        value: "\u{437}",
        legacy: false,
    },
    NamedRef {
        name: "zdot",
        value: "\u{17C}",
        legacy: false,
    },
    NamedRef {
        name: "zeetrf",
        value: "\u{2128}",
        legacy: false,
    },
    NamedRef {
        name: "zeta",
        value: "\u{3B6}",
        legacy: false,
    },
    NamedRef {
        name: "zfr",
        value: "\u{1D537}",
        legacy: false,
    },
    NamedRef {
        name: "zhcy",
        value: "\u{436}",
        legacy: false,
    },
    NamedRef {
        name: "zigrarr",
        value: "\u{21DD}",
        legacy: false,
    },
    NamedRef {
        name: "zopf",
        value: "\u{1D56B}",
        legacy: false,
    },
    NamedRef {
        name: "zscr",
        value: "\u{1D4CF}",
        legacy: false,
    },
    NamedRef {
        name: "zwj",
        value: "\u{200D}",
        legacy: false,
    },
    NamedRef {
        name: "zwnj",
        value: "\u{200C}",
        legacy: false,
    },
];
