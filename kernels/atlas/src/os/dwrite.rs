//! dwrite.dll: DirectWrite over the machine's fonts (`fonts.rs`, the host's). The factory gives the system font
//! collection (each host family, Windows' families found as their stand-ins), its families, fonts and font faces with
//! their metrics, character maps, glyph metrics and tables; font files made of a face's tables, through the system's
//! loader and its streams; rendering parameters; GDI interop for LOGFONTs; and glyph run analysis, which rasterizes a
//! glyph run's outlines into the alpha textures programs draw text with (grey, its three ClearType bytes alike, or
//! aliased). Text layout, text analysis, custom collections and loaders, and glyph outlines given to a geometry sink
//! are still stubs that say so.
//!
//! Every object has one vtable, of this DLL's exports, and answers for the interfaces it derives from with it.
//! DirectWrite passes floats by value in floating-point registers, which a host export reads itself (`f32_arg`), and
//! returns some (`return_f32`); the integer arguments after them are where each ISA's convention puts them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use exec::{FpRegs, Isa};
use kernels_core::lock;
use kernels_core::thread::Cx;
use kernels_core::user;

use super::com::{Guid, Object, com_add_ref, com_query_interface, com_release, guid};
use super::{Reply, dll};
use crate::fonts::{self, Face, Fonts};
use crate::process::Task;

dll! {
    "DWRITE.dll";
    host DWriteCreateFactory(3) => d_write_create_factory;
    // The methods of DirectWrite's objects, which their vtables point at.
    host AtlasComQueryInterface(3) => com_query_interface;
    host AtlasComAddRef(1) => com_add_ref;
    host AtlasComRelease(1) => com_release;
    host AtlasDwFactoryGetSystemFontCollection(3) => factory_get_system_font_collection;
    missing AtlasDwFactoryCreateCustomFontCollection;
    host AtlasDwFactoryRegisterFontCollectionLoader(2) => accept;
    host AtlasDwFactoryUnregisterFontCollectionLoader(2) => accept;
    missing AtlasDwFactoryCreateFontFileReference;
    missing AtlasDwFactoryCreateCustomFontFileReference;
    host AtlasDwFactoryCreateFontFace(7) => factory_create_font_face;
    host AtlasDwFactoryCreateRenderingParams(2) => factory_create_rendering_params;
    host AtlasDwFactoryCreateMonitorRenderingParams(3) => factory_create_monitor_rendering_params;
    host AtlasDwFactoryCreateCustomRenderingParams(7) => factory_create_custom_rendering_params;
    host AtlasDwFactoryRegisterFontFileLoader(2) => accept;
    host AtlasDwFactoryUnregisterFontFileLoader(2) => accept;
    host AtlasDwFactoryCreateTextFormat(9) => factory_create_text_format;
    missing AtlasDwFactoryCreateTypography;
    host AtlasDwFactoryGetGdiInterop(2) => factory_get_gdi_interop;
    host AtlasDwFactoryCreateTextLayout(7) => factory_create_text_layout;
    missing AtlasDwFactoryCreateGdiCompatibleTextLayout;
    missing AtlasDwFactoryCreateEllipsisTrimmingSign;
    missing AtlasDwFactoryCreateTextAnalyzer;
    missing AtlasDwFactoryCreateNumberSubstitution;
    host AtlasDwFactoryCreateGlyphRunAnalysis(9) => factory_create_glyph_run_analysis;
    host AtlasDwCollectionGetFontFamilyCount(1) => collection_get_font_family_count;
    host AtlasDwCollectionGetFontFamily(3) => collection_get_font_family;
    host AtlasDwCollectionFindFamilyName(4) => collection_find_family_name;
    host AtlasDwCollectionGetFontFromFontFace(3) => collection_get_font_from_font_face;
    host AtlasDwListGetFontCollection(2) => list_get_font_collection;
    host AtlasDwListGetFontCount(1) => list_get_font_count;
    host AtlasDwListGetFont(3) => list_get_font;
    host AtlasDwFamilyGetFamilyNames(2) => family_get_family_names;
    host AtlasDwFamilyGetFirstMatchingFont(5) => family_get_first_matching_font;
    host AtlasDwFamilyGetMatchingFonts(5) => family_get_matching_fonts;
    host AtlasDwFontGetFontFamily(2) => font_get_font_family;
    host AtlasDwFontGetWeight(1) => font_get_weight;
    host AtlasDwFontGetStretch(1) => font_get_stretch;
    host AtlasDwFontGetStyle(1) => font_get_style;
    host AtlasDwFontIsSymbolFont(1) => font_is_symbol_font;
    host AtlasDwFontGetFaceNames(2) => font_get_face_names;
    host AtlasDwFontGetInformationalStrings(4) => font_get_informational_strings;
    host AtlasDwFontGetSimulations(1) => font_get_simulations;
    host AtlasDwFontGetMetrics(2) => font_get_metrics;
    host AtlasDwFontHasCharacter(3) => font_has_character;
    host AtlasDwFontCreateFontFace(2) => font_create_font_face;
    host AtlasDwStringsGetCount(1) => strings_get_count;
    host AtlasDwStringsFindLocaleName(4) => strings_find_locale_name;
    host AtlasDwStringsGetLocaleNameLength(3) => strings_get_locale_name_length;
    host AtlasDwStringsGetLocaleName(4) => strings_get_locale_name;
    host AtlasDwStringsGetStringLength(3) => strings_get_string_length;
    host AtlasDwStringsGetString(4) => strings_get_string;
    host AtlasDwFaceGetType(1) => face_get_type;
    host AtlasDwFaceGetFiles(3) => face_get_files;
    host AtlasDwFaceGetIndex(1) => zero;
    host AtlasDwFaceGetSimulations(1) => face_get_simulations;
    host AtlasDwFaceIsSymbolFont(1) => face_is_symbol_font;
    host AtlasDwFaceGetMetrics(2) => face_get_metrics;
    host AtlasDwFaceGetGlyphCount(1) => face_get_glyph_count;
    host AtlasDwFaceGetDesignGlyphMetrics(5) => face_get_design_glyph_metrics;
    host AtlasDwFaceGetGlyphIndices(4) => face_get_glyph_indices;
    host AtlasDwFaceTryGetFontTable(6) => face_try_get_font_table;
    host AtlasDwFaceReleaseFontTable(2) => zero;
    missing AtlasDwFaceGetGlyphRunOutline;
    host AtlasDwFaceGetRecommendedRenderingMode(6) => face_get_recommended_rendering_mode;
    host AtlasDwFaceGetGdiCompatibleMetrics(5) => face_get_gdi_compatible_metrics;
    host AtlasDwFaceGetGdiCompatibleGlyphMetrics(9) => face_get_gdi_compatible_glyph_metrics;
    host AtlasDwFileGetReferenceKey(3) => file_get_reference_key;
    host AtlasDwFileGetLoader(2) => file_get_loader;
    host AtlasDwFileAnalyze(5) => file_analyze;
    host AtlasDwLoaderCreateStreamFromKey(4) => loader_create_stream_from_key;
    host AtlasDwStreamReadFileFragment(5) => stream_read_file_fragment;
    host AtlasDwStreamReleaseFileFragment(2) => zero;
    host AtlasDwStreamGetFileSize(2) => stream_get_file_size;
    host AtlasDwStreamGetLastWriteTime(2) => stream_get_last_write_time;
    host AtlasDwParamsGetGamma(1) => params_get_gamma;
    host AtlasDwParamsGetEnhancedContrast(1) => params_get_enhanced_contrast;
    host AtlasDwParamsGetClearTypeLevel(1) => params_get_clear_type_level;
    host AtlasDwParamsGetPixelGeometry(1) => params_get_pixel_geometry;
    host AtlasDwParamsGetRenderingMode(1) => params_get_rendering_mode;
    host AtlasDwAnalysisGetAlphaTextureBounds(3) => analysis_get_alpha_texture_bounds;
    host AtlasDwAnalysisCreateAlphaTexture(5) => analysis_create_alpha_texture;
    host AtlasDwAnalysisGetAlphaBlendParams(5) => analysis_get_alpha_blend_params;
    host AtlasDwFormatSetTextAlignment(2) => format_set_text_alignment;
    host AtlasDwFormatSetParagraphAlignment(2) => format_set_paragraph_alignment;
    host AtlasDwFormatSetWordWrapping(2) => format_set_word_wrapping;
    host AtlasDwFormatSetReadingDirection(2) => format_set_reading_direction;
    host AtlasDwFormatSetFlowDirection(2) => format_set_flow_direction;
    host AtlasDwFormatSetIncrementalTabStop(2) => format_set_incremental_tab_stop;
    host AtlasDwFormatSetTrimming(3) => format_set_trimming;
    host AtlasDwFormatSetLineSpacing(4) => format_set_line_spacing;
    host AtlasDwFormatGetTextAlignment(1) => format_get_text_alignment;
    host AtlasDwFormatGetParagraphAlignment(1) => format_get_paragraph_alignment;
    host AtlasDwFormatGetWordWrapping(1) => format_get_word_wrapping;
    host AtlasDwFormatGetReadingDirection(1) => format_get_reading_direction;
    host AtlasDwFormatGetFlowDirection(1) => format_get_flow_direction;
    host AtlasDwFormatGetIncrementalTabStop(1) => format_get_incremental_tab_stop;
    host AtlasDwFormatGetTrimming(3) => format_get_trimming;
    host AtlasDwFormatGetLineSpacing(4) => format_get_line_spacing;
    host AtlasDwFormatGetFontCollection(2) => list_get_font_collection;
    host AtlasDwFormatGetFontFamilyNameLength(1) => format_get_font_family_name_length;
    host AtlasDwFormatGetFontFamilyName(3) => format_get_font_family_name;
    host AtlasDwFormatGetFontWeight(1) => format_get_font_weight;
    host AtlasDwFormatGetFontStyle(1) => format_get_font_style;
    host AtlasDwFormatGetFontStretch(1) => format_get_font_stretch;
    host AtlasDwFormatGetFontSize(1) => format_get_font_size;
    host AtlasDwFormatGetLocaleNameLength(1) => format_get_locale_name_length;
    host AtlasDwFormatGetLocaleName(3) => format_get_locale_name;
    host AtlasDwLayoutSetMaxWidth(2) => layout_set_max_width;
    host AtlasDwLayoutSetMaxHeight(2) => layout_set_max_height;
    missing AtlasDwLayoutSetFontCollection;
    missing AtlasDwLayoutSetFontFamilyName;
    missing AtlasDwLayoutSetFontWeight;
    missing AtlasDwLayoutSetFontStyle;
    missing AtlasDwLayoutSetFontStretch;
    missing AtlasDwLayoutSetFontSize;
    missing AtlasDwLayoutSetUnderline;
    missing AtlasDwLayoutSetStrikethrough;
    missing AtlasDwLayoutSetDrawingEffect;
    missing AtlasDwLayoutSetInlineObject;
    missing AtlasDwLayoutSetTypography;
    missing AtlasDwLayoutSetLocaleName;
    host AtlasDwLayoutGetMaxWidth(1) => layout_get_max_width;
    host AtlasDwLayoutGetMaxHeight(1) => layout_get_max_height;
    missing AtlasDwLayoutGetFontCollection2;
    missing AtlasDwLayoutGetFontFamilyNameLength2;
    missing AtlasDwLayoutGetFontFamilyName2;
    missing AtlasDwLayoutGetFontWeight2;
    missing AtlasDwLayoutGetFontStyle2;
    missing AtlasDwLayoutGetFontStretch2;
    missing AtlasDwLayoutGetFontSize2;
    missing AtlasDwLayoutGetUnderline;
    missing AtlasDwLayoutGetStrikethrough;
    missing AtlasDwLayoutGetDrawingEffect;
    missing AtlasDwLayoutGetInlineObject;
    missing AtlasDwLayoutGetTypography;
    missing AtlasDwLayoutGetLocaleNameLength2;
    missing AtlasDwLayoutGetLocaleName2;
    missing AtlasDwLayoutDraw;
    host AtlasDwLayoutGetLineMetrics(4) => layout_get_line_metrics;
    host AtlasDwLayoutGetMetrics(2) => layout_get_metrics;
    host AtlasDwLayoutGetOverhangMetrics(2) => layout_get_overhang_metrics;
    host AtlasDwLayoutGetClusterMetrics(4) => layout_get_cluster_metrics;
    host AtlasDwLayoutDetermineMinWidth(2) => layout_determine_min_width;
    host AtlasDwLayoutHitTestPoint(6) => layout_hit_test_point;
    host AtlasDwLayoutHitTestTextPosition(6) => layout_hit_test_text_position;
    host AtlasDwLayoutHitTestTextRange(8) => layout_hit_test_text_range;
    host AtlasDwInteropCreateFontFromLOGFONT(3) => interop_create_font_from_logfont;
    host AtlasDwInteropConvertFontToLOGFONT(4) => interop_convert_font_to_logfont;
    host AtlasDwInteropConvertFontFaceToLOGFONT(3) => interop_convert_font_face_to_logfont;
    missing AtlasDwInteropCreateFontFaceFromHdc;
    missing AtlasDwInteropCreateBitmapRenderTarget;
}

const FACTORY: [&str; 24] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwFactoryGetSystemFontCollection",
    "AtlasDwFactoryCreateCustomFontCollection", "AtlasDwFactoryRegisterFontCollectionLoader",
    "AtlasDwFactoryUnregisterFontCollectionLoader", "AtlasDwFactoryCreateFontFileReference",
    "AtlasDwFactoryCreateCustomFontFileReference", "AtlasDwFactoryCreateFontFace",
    "AtlasDwFactoryCreateRenderingParams", "AtlasDwFactoryCreateMonitorRenderingParams",
    "AtlasDwFactoryCreateCustomRenderingParams", "AtlasDwFactoryRegisterFontFileLoader",
    "AtlasDwFactoryUnregisterFontFileLoader", "AtlasDwFactoryCreateTextFormat", "AtlasDwFactoryCreateTypography",
    "AtlasDwFactoryGetGdiInterop", "AtlasDwFactoryCreateTextLayout", "AtlasDwFactoryCreateGdiCompatibleTextLayout",
    "AtlasDwFactoryCreateEllipsisTrimmingSign", "AtlasDwFactoryCreateTextAnalyzer",
    "AtlasDwFactoryCreateNumberSubstitution", "AtlasDwFactoryCreateGlyphRunAnalysis",
];
const COLLECTION: [&str; 7] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwCollectionGetFontFamilyCount",
    "AtlasDwCollectionGetFontFamily", "AtlasDwCollectionFindFamilyName", "AtlasDwCollectionGetFontFromFontFace",
];
const FAMILY: [&str; 9] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwListGetFontCollection",
    "AtlasDwListGetFontCount", "AtlasDwListGetFont", "AtlasDwFamilyGetFamilyNames",
    "AtlasDwFamilyGetFirstMatchingFont", "AtlasDwFamilyGetMatchingFonts",
];
const LIST: [&str; 6] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwListGetFontCollection",
    "AtlasDwListGetFontCount", "AtlasDwListGetFont",
];
const FONT: [&str; 14] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwFontGetFontFamily", "AtlasDwFontGetWeight",
    "AtlasDwFontGetStretch", "AtlasDwFontGetStyle", "AtlasDwFontIsSymbolFont", "AtlasDwFontGetFaceNames",
    "AtlasDwFontGetInformationalStrings", "AtlasDwFontGetSimulations", "AtlasDwFontGetMetrics",
    "AtlasDwFontHasCharacter", "AtlasDwFontCreateFontFace",
];
const STRINGS: [&str; 9] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwStringsGetCount",
    "AtlasDwStringsFindLocaleName", "AtlasDwStringsGetLocaleNameLength", "AtlasDwStringsGetLocaleName",
    "AtlasDwStringsGetStringLength", "AtlasDwStringsGetString",
];
const FACE: [&str; 18] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwFaceGetType", "AtlasDwFaceGetFiles",
    "AtlasDwFaceGetIndex", "AtlasDwFaceGetSimulations", "AtlasDwFaceIsSymbolFont", "AtlasDwFaceGetMetrics",
    "AtlasDwFaceGetGlyphCount", "AtlasDwFaceGetDesignGlyphMetrics", "AtlasDwFaceGetGlyphIndices",
    "AtlasDwFaceTryGetFontTable", "AtlasDwFaceReleaseFontTable", "AtlasDwFaceGetGlyphRunOutline",
    "AtlasDwFaceGetRecommendedRenderingMode", "AtlasDwFaceGetGdiCompatibleMetrics",
    "AtlasDwFaceGetGdiCompatibleGlyphMetrics",
];
const FILE: [&str; 6] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwFileGetReferenceKey",
    "AtlasDwFileGetLoader", "AtlasDwFileAnalyze",
];
const LOADER: [&str; 4] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwLoaderCreateStreamFromKey",
];
const STREAM: [&str; 7] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwStreamReadFileFragment",
    "AtlasDwStreamReleaseFileFragment", "AtlasDwStreamGetFileSize", "AtlasDwStreamGetLastWriteTime",
];
const PARAMS: [&str; 8] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwParamsGetGamma",
    "AtlasDwParamsGetEnhancedContrast", "AtlasDwParamsGetClearTypeLevel", "AtlasDwParamsGetPixelGeometry",
    "AtlasDwParamsGetRenderingMode",
];
const ANALYSIS: [&str; 6] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwAnalysisGetAlphaTextureBounds",
    "AtlasDwAnalysisCreateAlphaTexture", "AtlasDwAnalysisGetAlphaBlendParams",
];
const FORMAT: [&str; 28] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwFormatSetTextAlignment",
    "AtlasDwFormatSetParagraphAlignment", "AtlasDwFormatSetWordWrapping", "AtlasDwFormatSetReadingDirection",
    "AtlasDwFormatSetFlowDirection", "AtlasDwFormatSetIncrementalTabStop", "AtlasDwFormatSetTrimming",
    "AtlasDwFormatSetLineSpacing", "AtlasDwFormatGetTextAlignment", "AtlasDwFormatGetParagraphAlignment",
    "AtlasDwFormatGetWordWrapping", "AtlasDwFormatGetReadingDirection", "AtlasDwFormatGetFlowDirection",
    "AtlasDwFormatGetIncrementalTabStop", "AtlasDwFormatGetTrimming", "AtlasDwFormatGetLineSpacing",
    "AtlasDwFormatGetFontCollection", "AtlasDwFormatGetFontFamilyNameLength", "AtlasDwFormatGetFontFamilyName",
    "AtlasDwFormatGetFontWeight", "AtlasDwFormatGetFontStyle", "AtlasDwFormatGetFontStretch",
    "AtlasDwFormatGetFontSize", "AtlasDwFormatGetLocaleNameLength", "AtlasDwFormatGetLocaleName",
];
const LAYOUT: [&str; 67] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwFormatSetTextAlignment",
    "AtlasDwFormatSetParagraphAlignment", "AtlasDwFormatSetWordWrapping", "AtlasDwFormatSetReadingDirection",
    "AtlasDwFormatSetFlowDirection", "AtlasDwFormatSetIncrementalTabStop", "AtlasDwFormatSetTrimming",
    "AtlasDwFormatSetLineSpacing", "AtlasDwFormatGetTextAlignment", "AtlasDwFormatGetParagraphAlignment",
    "AtlasDwFormatGetWordWrapping", "AtlasDwFormatGetReadingDirection", "AtlasDwFormatGetFlowDirection",
    "AtlasDwFormatGetIncrementalTabStop", "AtlasDwFormatGetTrimming", "AtlasDwFormatGetLineSpacing",
    "AtlasDwFormatGetFontCollection", "AtlasDwFormatGetFontFamilyNameLength", "AtlasDwFormatGetFontFamilyName",
    "AtlasDwFormatGetFontWeight", "AtlasDwFormatGetFontStyle", "AtlasDwFormatGetFontStretch",
    "AtlasDwFormatGetFontSize", "AtlasDwFormatGetLocaleNameLength", "AtlasDwFormatGetLocaleName",
    "AtlasDwLayoutSetMaxWidth", "AtlasDwLayoutSetMaxHeight", "AtlasDwLayoutSetFontCollection",
    "AtlasDwLayoutSetFontFamilyName", "AtlasDwLayoutSetFontWeight", "AtlasDwLayoutSetFontStyle",
    "AtlasDwLayoutSetFontStretch", "AtlasDwLayoutSetFontSize", "AtlasDwLayoutSetUnderline",
    "AtlasDwLayoutSetStrikethrough", "AtlasDwLayoutSetDrawingEffect", "AtlasDwLayoutSetInlineObject",
    "AtlasDwLayoutSetTypography", "AtlasDwLayoutSetLocaleName", "AtlasDwLayoutGetMaxWidth",
    "AtlasDwLayoutGetMaxHeight", "AtlasDwLayoutGetFontCollection2", "AtlasDwLayoutGetFontFamilyNameLength2",
    "AtlasDwLayoutGetFontFamilyName2", "AtlasDwLayoutGetFontWeight2", "AtlasDwLayoutGetFontStyle2",
    "AtlasDwLayoutGetFontStretch2", "AtlasDwLayoutGetFontSize2", "AtlasDwLayoutGetUnderline",
    "AtlasDwLayoutGetStrikethrough", "AtlasDwLayoutGetDrawingEffect", "AtlasDwLayoutGetInlineObject",
    "AtlasDwLayoutGetTypography", "AtlasDwLayoutGetLocaleNameLength2", "AtlasDwLayoutGetLocaleName2",
    "AtlasDwLayoutDraw", "AtlasDwLayoutGetLineMetrics", "AtlasDwLayoutGetMetrics", "AtlasDwLayoutGetOverhangMetrics",
    "AtlasDwLayoutGetClusterMetrics", "AtlasDwLayoutDetermineMinWidth", "AtlasDwLayoutHitTestPoint",
    "AtlasDwLayoutHitTestTextPosition", "AtlasDwLayoutHitTestTextRange",
];
const INTEROP: [&str; 8] = [
    "AtlasComQueryInterface", "AtlasComAddRef", "AtlasComRelease", "AtlasDwInteropCreateFontFromLOGFONT",
    "AtlasDwInteropConvertFontToLOGFONT", "AtlasDwInteropConvertFontFaceToLOGFONT",
    "AtlasDwInteropCreateFontFaceFromHdc", "AtlasDwInteropCreateBitmapRenderTarget",
];

const S_OK: u64 = 0;
const E_INVALIDARG: u64 = 0x8007_0057;
const E_POINTER: u64 = 0x8000_4003;
const E_OUTOFMEMORY: u64 = 0x8007_000e;
const E_NOT_SUFFICIENT_BUFFER: u64 = 0x8007_007a;
/// DWRITE_E_NOFONT: no font matches.
const DWRITE_E_NOFONT: u64 = 0x8898_5002;
/// DWRITE_E_FILEFORMAT: a font file that isn't one of the machine's.
const DWRITE_E_FILEFORMAT: u64 = 0x8898_5000;

const IID_FACTORY: Guid = guid(0xb859_ee5a, 0xd838, 0x4b5b, [0xa2, 0xe8, 0x1a, 0xdc, 0x7d, 0x93, 0xdb, 0x48]);
const IID_COLLECTION: Guid = guid(0xa84c_ee02, 0x3eea, 0x4eee, [0xa8, 0x27, 0x87, 0xc1, 0xa0, 0x2a, 0x0f, 0xcc]);
const IID_FAMILY: Guid = guid(0xda20_d8ef, 0x812a, 0x4c43, [0x98, 0x02, 0x62, 0xec, 0x4a, 0xbd, 0x7a, 0xdd]);
const IID_LIST: Guid = guid(0x1a0d_8438, 0x1d97, 0x4ec1, [0xae, 0xf9, 0xa2, 0xfb, 0x86, 0xed, 0x6a, 0xcb]);
const IID_FONT: Guid = guid(0xacd1_6696, 0x8c14, 0x4f5d, [0x87, 0x7e, 0xfe, 0x3f, 0xc1, 0xd3, 0x27, 0x37]);
const IID_FACE: Guid = guid(0x5f49_804d, 0x7024, 0x4d43, [0xbf, 0xa9, 0xd2, 0x59, 0x84, 0xf5, 0x38, 0x49]);
const IID_STRINGS: Guid = guid(0x0825_6209, 0x099a, 0x4b34, [0xb8, 0x6d, 0xc2, 0x2b, 0x11, 0x0e, 0x77, 0x71]);
const IID_FILE: Guid = guid(0x739d_886a, 0xcef5, 0x47dc, [0x87, 0x69, 0x1a, 0x8b, 0x41, 0xbe, 0xbb, 0xb0]);
const IID_LOADER: Guid = guid(0x727c_ad4e, 0xd6af, 0x4c9e, [0x8a, 0x08, 0xd6, 0x95, 0xb1, 0x1c, 0xaa, 0x49]);
const IID_STREAM: Guid = guid(0x6d48_65fe, 0x0ab8, 0x4d91, [0x8f, 0x62, 0x5d, 0xd6, 0xbe, 0x34, 0xa3, 0xe0]);
const IID_PARAMS: Guid = guid(0x2f0d_a53a, 0x2add, 0x47cd, [0x82, 0xee, 0xd9, 0xec, 0x34, 0x68, 0x8e, 0x75]);
const IID_ANALYSIS: Guid = guid(0x7d97_dbf7, 0xe085, 0x42d4, [0x81, 0xe3, 0x6a, 0x88, 0x3b, 0xde, 0xd1, 0x18]);
const IID_FORMAT: Guid = guid(0x9c90_6818, 0x31d7, 0x4fd3, [0xa1, 0x51, 0x7c, 0x5e, 0x22, 0x5d, 0xb5, 0x5a]);
const IID_LAYOUT: Guid = guid(0x5373_7037, 0x6d14, 0x410b, [0x9b, 0xfe, 0x0b, 0x18, 0x2b, 0xb7, 0x09, 0x61]);
const IID_INTEROP: Guid = guid(0x1edd_9491, 0x9853, 0x4299, [0x89, 0x8f, 0x64, 0x32, 0x98, 0x3b, 0x6f, 0x3a]);

/// A DirectWrite object of atlas's: what it is, by the machine's font list's numbers.
#[derive(Debug, Clone)]
pub enum Dw {
    Factory,
    /// The system font collection.
    Collection,
    Family(usize),
    List(Vec<usize>),
    Font(usize),
    Strings(Vec<(String, String)>),
    Face(usize),
    File(usize),
    Loader,
    Stream(usize),
    Params { gamma: f32, contrast: f32, clear_type: f32, geometry: u32, mode: u32 },
    Analysis { left: i32, top: i32, width: usize, height: usize, alpha: Arc<Vec<u8>>, aliased: bool },
    Interop,
    Format(Box<Format>),
    Layout(Box<Layout>),
}

/// A text format: the font and paragraph a layout starts from.
#[derive(Debug, Clone)]
pub struct Format {
    family: String,
    weight: u16,
    style: u32,
    stretch: u16,
    size: f32,
    locale: String,
    alignment: u32,
    paragraph: u32,
    wrapping: u32,
    reading: u32,
    flow: u32,
    tab: f32,
    /// DWRITE_TRIMMING: granularity, delimiter, delimiter count.
    trimming: [u32; 3],
    /// DWRITE_LINE_SPACING_METHOD, then the spacing and the baseline (uniform spacing only).
    spacing: (u32, f32, f32),
}

/// A text layout: a format, its text, its box, and its lines as laid out.
#[derive(Debug, Clone)]
pub struct Layout {
    format: Format,
    text: Vec<u16>,
    max_width: f32,
    max_height: f32,
    lines: Vec<Line>,
    /// Each cluster's start in the text, its length (1, or 2 for a surrogate pair) and its advance.
    clusters: Vec<(usize, usize, f32)>,
    line_height: f32,
    baseline: f32,
}

/// A line: its first cluster, how many, how many of those are trailing whitespace and how many a newline, and its
/// width without and with its trailing whitespace.
#[derive(Debug, Clone, Copy, Default)]
struct Line {
    first: usize,
    count: usize,
    trailing: usize,
    newline: usize,
    width: f32,
    width_ws: f32,
}

/// Whether an object answers for an interface (IUnknown aside).
pub fn answers(dw: &Dw, iid: &Guid) -> bool {
    match dw {
        Dw::Factory => *iid == IID_FACTORY,
        Dw::Collection => *iid == IID_COLLECTION,
        Dw::Family(_) => *iid == IID_FAMILY || *iid == IID_LIST,
        Dw::List(_) => *iid == IID_LIST,
        Dw::Font(_) => *iid == IID_FONT,
        Dw::Strings(_) => *iid == IID_STRINGS,
        Dw::Face(_) => *iid == IID_FACE,
        Dw::File(_) => *iid == IID_FILE,
        Dw::Loader => *iid == IID_LOADER,
        Dw::Stream(_) => *iid == IID_STREAM,
        Dw::Params { .. } => *iid == IID_PARAMS,
        Dw::Analysis { .. } => *iid == IID_ANALYSIS,
        Dw::Interop => *iid == IID_INTEROP,
        Dw::Format(_) => *iid == IID_FORMAT,
        Dw::Layout(_) => *iid == IID_LAYOUT || *iid == IID_FORMAT,
    }
}

fn table_of(dw: &Dw) -> &'static [&'static str] {
    match dw {
        Dw::Factory => &FACTORY,
        Dw::Collection => &COLLECTION,
        Dw::Family(_) => &FAMILY,
        Dw::List(_) => &LIST,
        Dw::Font(_) => &FONT,
        Dw::Strings(_) => &STRINGS,
        Dw::Face(_) => &FACE,
        Dw::File(_) => &FILE,
        Dw::Loader => &LOADER,
        Dw::Stream(_) => &STREAM,
        Dw::Params { .. } => &PARAMS,
        Dw::Analysis { .. } => &ANALYSIS,
        Dw::Interop => &INTEROP,
        Dw::Format(_) => &FORMAT,
        Dw::Layout(_) => &LAYOUT,
    }
}

/// What a process's font tables and files are copied to in its memory, read-only to it and kept while it runs:
/// (process, face, tag or 0 for the file) to the copy's address.
static COPIES: Mutex<Option<HashMap<(u64, usize, u32), u64>>> = Mutex::new(None);

/// A process that's over has no copies.
pub fn forget(pid: u64) {
    if let Some(map) = lock(&COPIES).as_mut() {
        map.retain(|k, _| k.0 != pid);
    }
}

impl Task {
    fn fonts(&self) -> &'static Fonts {
        fonts::get(&self.k.host)
    }

    fn face(&self, index: usize) -> Option<Arc<Face>> {
        self.fonts().open(&self.k.host, index)
    }

    /// A new object, written to `out`: S_OK, or E_OUTOFMEMORY.
    fn give(&mut self, cx: &mut Cx<'_>, out: u64, dw: Dw) -> Reply {
        if out == 0 {
            return Reply::Value(E_POINTER);
        }
        let table = table_of(&dw);
        match self.make_object(cx, "dwrite.dll", &[table], Object::Dwrite(dw)) {
            Ok(0) => {
                let _ = self.write_ptr(cx, out, 0);
                Reply::Value(E_OUTOFMEMORY)
            }
            Ok(at) => {
                let _ = self.write_ptr(cx, out, at);
                Reply::Value(S_OK)
            }
            Err(reply) => reply,
        }
    }

    fn of(&self, this: u64) -> Option<Dw> {
        self.dw(this, |d| d.clone())
    }

    fn dw_put(&mut self, cx: &mut Cx<'_>, at: u64, bytes: &[u8]) -> bool {
        at != 0 && self.mem(cx, true, |t| user::write(&mut t.space(), at, bytes)).is_ok()
    }

    fn dw_put_u32(&mut self, cx: &mut Cx<'_>, at: u64, v: u32) -> bool {
        self.dw_put(cx, at, &v.to_le_bytes())
    }

    /// A copy of `bytes` in the process's memory, made once for each face and tag.
    fn copy(&mut self, cx: &mut Cx<'_>, face: usize, tag: u32, bytes: &[u8]) -> Option<u64> {
        let key = (self.process.pid, face, tag);
        if let Some(&at) = lock(&COPIES).get_or_insert_with(HashMap::new).get(&key) {
            return Some(at);
        }
        let at = self.virtual_alloc(cx, 0, (bytes.len() as u64).max(1), 0x3000, 4)?;
        self.dw_put(cx, at, bytes);
        lock(&COPIES).get_or_insert_with(HashMap::new).insert(key, at);
        Some(at)
    }

    /// The floating-point argument at `index` (counting every argument, `this` too), which is `float` among the
    /// floating-point ones: in xmm`index` on x86-64 (on the stack past the fourth), in v`float` on ARM64, on the stack
    /// on x86.
    fn f32_arg(&mut self, cx: &mut Cx<'_>, a: &[u64], index: usize, float: usize) -> f32 {
        let fp = cx.turn(|t| t.thread().fp());
        match (self.isa, fp) {
            (Isa::X86_64, FpRegs::X86(f)) if index < 4 => {
                let at = 160 + 16 * index;
                f32::from_le_bytes([f.fxsave[at], f.fxsave[at + 1], f.fxsave[at + 2], f.fxsave[at + 3]])
            }
            (Isa::Arm64, FpRegs::Arm64(f)) => f32::from_bits(f.v[float] as u32),
            _ => f32::from_bits(a.get(index).copied().unwrap_or(0) as u32),
        }
    }

    /// An integer argument that comes after floating-point ones: at `index` on x86-64 and x86 (every argument has a
    /// place), at `arm64` on ARM64 (whose floats have registers of their own).
    fn int_arg(&self, a: &[u64], index: usize, arm64: usize) -> u64 {
        let at = if self.isa == Isa::Arm64 { arm64 } else { index };
        a.get(at).copied().unwrap_or(0)
    }

    /// Returns a float: in xmm0 on x86-64, s0 on ARM64. (x86 returns it on the x87 stack, which isn't done here.)
    fn return_f32(&mut self, cx: &mut Cx<'_>, v: f32) -> Reply {
        let isa = self.isa;
        cx.turn(|t| {
            let mut fp = t.thread().fp();
            match (isa, &mut fp) {
                (Isa::X86_64, FpRegs::X86(f)) => f.fxsave[160..164].copy_from_slice(&v.to_le_bytes()),
                (Isa::Arm64, FpRegs::Arm64(f)) => f.v[0] = u128::from(v.to_bits()),
                _ => {}
            }
            t.thread().set_fp(&fp);
        });
        Reply::Value(u64::from(v.to_bits()))
    }

    /// A pointer the program has at `at`.
    fn get_ptr_at(&mut self, cx: &mut Cx<'_>, at: u64) -> u64 {
        let abi = self.abi();
        let b = self.mem(cx, false, |t| user::read_vec(&t.space(), at, abi.width as usize)).unwrap_or_default();
        abi.ptr_at(&b, 0)
    }

    fn read_f32s(&mut self, cx: &mut Cx<'_>, at: u64, n: usize) -> Vec<f32> {
        let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), at, n * 4)).unwrap_or_default();
        bytes.chunks(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
    }

    fn read_u16s(&mut self, cx: &mut Cx<'_>, at: u64, n: usize) -> Vec<u16> {
        let bytes = self.mem(cx, false, |t| user::read_vec(&t.space(), at, n * 2)).unwrap_or_default();
        bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    }
}

fn accept(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(S_OK)
}

fn zero(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// `DWriteCreateFactory(type, &iid, &factory)`: shared or isolated, the same.
fn d_write_create_factory(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let iid = task.read_guid(cx, a[1]);
    if iid.is_none_or(|i| i != IID_FACTORY && i != [0; 16] && i != super::com::guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46])) {
        if a[2] != 0 {
            let _ = task.write_ptr(cx, a[2], 0);
        }
        return Reply::Value(0x8000_4002);
    }
    task.give(cx, a[2], Dw::Factory)
}

// ---- The factory ----

fn factory_get_system_font_collection(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.give(cx, a[1], Dw::Collection)
}

/// `CreateFontFace(type, count, &files, index, simulations, &face)`: from one of the machine's font files.
fn factory_create_font_face(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let first = if a[1] >= 1 && a[2] != 0 { task.get_ptr_at(cx, a[2]) } else { 0 };
    match task.of(first) {
        Some(Dw::File(face)) => task.give(cx, a[6], Dw::Face(face)),
        _ => {
            let _ = task.write_ptr(cx, a[6], 0);
            Reply::Value(DWRITE_E_FILEFORMAT)
        }
    }
}

/// The parameters Windows' defaults are: gamma 1.8, enhanced contrast 0.5, full ClearType, RGB, the default mode.
fn default_params() -> Dw {
    Dw::Params { gamma: 1.8, contrast: 0.5, clear_type: 1.0, geometry: 1, mode: 0 }
}

fn factory_create_rendering_params(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.give(cx, a[1], default_params())
}

fn factory_create_monitor_rendering_params(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.give(cx, a[2], default_params())
}

/// `CreateCustomRenderingParams(gamma, contrast, clear_type, geometry, mode, &params)`.
fn factory_create_custom_rendering_params(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (gamma, contrast, clear_type) = (task.f32_arg(cx, a, 1, 0), task.f32_arg(cx, a, 2, 1), task.f32_arg(cx, a, 3, 2));
    let (geometry, mode, out) = (task.int_arg(a, 4, 1) as u32, task.int_arg(a, 5, 2) as u32, task.int_arg(a, 6, 3));
    task.give(cx, out, Dw::Params { gamma, contrast, clear_type, geometry, mode })
}

fn factory_get_gdi_interop(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.give(cx, a[1], Dw::Interop)
}

/// DWRITE_RENDERING_MODE_ALIASED and _OUTLINE.
const MODE_ALIASED: u32 = 1;
const MODE_OUTLINE: u32 = 6;

/// `CreateGlyphRunAnalysis(&run, pixels_per_dip, &transform, mode, measuring, x, y, &analysis)`: the run rasterized
/// now, its glyphs at their advances and offsets from the baseline's origin, in pixels (DIPs times `pixels_per_dip`)
/// through the transform.
fn factory_create_glyph_run_analysis(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let ppd = task.f32_arg(cx, a, 2, 0);
    let (transform_at, mode, out) = (task.int_arg(a, 3, 2), task.int_arg(a, 4, 3) as u32, task.int_arg(a, 8, 5));
    let (x, y) = (task.f32_arg(cx, a, 6, 1), task.f32_arg(cx, a, 7, 2));
    if mode == 0 || mode == MODE_OUTLINE || a[1] == 0 {
        let _ = task.write_ptr(cx, out, 0);
        return Reply::Value(E_INVALIDARG);
    }
    // DWRITE_GLYPH_RUN: the face, the em size, the count, then the indices', advances' and offsets' arrays.
    let w = task.abi().width;
    let run = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], (6 * w) as usize)).unwrap_or_default();
    let ptr = |i: u64| task.abi().ptr_at(&run, i as usize);
    let face_ptr = ptr(0);
    let em = f32::from_le_bytes(run[w as usize..w as usize + 4].try_into().unwrap_or_default());
    let count = u32::from_le_bytes(run[w as usize + 4..w as usize + 8].try_into().unwrap_or_default()) as usize;
    let (indices_at, advances_at, offsets_at) = (ptr(w + 8), ptr(2 * w + 8), ptr(3 * w + 8));
    let Some(Dw::Face(face_index)) = task.of(face_ptr) else { return Reply::Value(E_INVALIDARG) };
    let Some(face) = task.face(face_index) else { return Reply::Value(E_INVALIDARG) };
    let count = count.min(4096);
    let indices = task.read_u16s(cx, indices_at, count);
    let advances = if advances_at != 0 { task.read_f32s(cx, advances_at, count) } else { Vec::new() };
    let offsets = if offsets_at != 0 { task.read_f32s(cx, offsets_at, 2 * count) } else { Vec::new() };
    let transform = if transform_at != 0 {
        let m = task.read_f32s(cx, transform_at, 6);
        [m[0], m[1], m[2], m[3], m[4], m[5]]
    } else {
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
    };
    let ppd = if ppd > 0.0 { ppd } else { 1.0 };
    let scale = em / f32::from(face.metrics.units_per_em);
    let mut steps = Vec::new();
    let mut pen = 0f32;
    for (i, &g) in indices.iter().enumerate() {
        let (dx, dy) = if offsets.len() >= 2 * i + 2 { (offsets[2 * i], offsets[2 * i + 1]) } else { (0.0, 0.0) };
        let origin = ((x + pen + dx) * ppd, (y - dy) * ppd);
        steps.extend(face.steps(g, scale * ppd, transform, origin));
        pen += advances.get(i).copied().unwrap_or_else(|| f32::from(face.h_metrics(g).0) * scale);
    }
    let analysis = match display::raster::bounds(&steps) {
        Some((l, t, r, b)) => {
            let (width, height) = ((r - l) as usize, (b - t) as usize);
            let alpha = if width * height <= 1 << 24 { display::raster::fill(&steps, l, t, width, height) } else { Vec::new() };
            Dw::Analysis { left: l, top: t, width, height, alpha: Arc::new(alpha), aliased: mode == MODE_ALIASED }
        }
        None => Dw::Analysis { left: 0, top: 0, width: 0, height: 0, alpha: Arc::new(Vec::new()), aliased: mode == MODE_ALIASED },
    };
    task.give(cx, out, analysis)
}

// ---- The system font collection, its families and lists ----

fn collection_get_font_family_count(task: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(task.fonts().families.len() as u64)
}

fn collection_get_font_family(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let index = a[1] as u32 as usize;
    if index >= task.fonts().families.len() {
        let _ = task.write_ptr(cx, a[2], 0);
        return Reply::Value(E_INVALIDARG);
    }
    task.give(cx, a[2], Dw::Family(index))
}

/// `FindFamilyName(name, &index, &exists)`: UINT_MAX and FALSE for one there isn't.
fn collection_find_family_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let name = task.text_arg(cx, a[1], true);
    let found = task.fonts().find(&name);
    task.dw_put_u32(cx, a[2], found.map_or(u32::MAX, |i| i as u32));
    task.dw_put_u32(cx, a[3], u32::from(found.is_some()));
    Reply::Value(S_OK)
}

fn collection_get_font_from_font_face(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    match task.of(a[1]) {
        Some(Dw::Face(face)) => task.give(cx, a[2], Dw::Font(face)),
        _ => {
            let _ = task.write_ptr(cx, a[2], 0);
            Reply::Value(DWRITE_E_NOFONT)
        }
    }
}

/// A family's or a list's faces.
fn faces_of(task: &Task, dw: &Dw) -> Vec<usize> {
    match dw {
        Dw::Family(f) => task.fonts().families.get(*f).map(|f| f.faces.clone()).unwrap_or_default(),
        Dw::List(faces) => faces.clone(),
        _ => Vec::new(),
    }
}

fn list_get_font_collection(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.give(cx, a[1], Dw::Collection)
}

fn list_get_font_count(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let dw = task.of(a[0]);
    Reply::Value(dw.map_or(0, |d| faces_of(task, &d).len() as u64))
}

fn list_get_font(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let faces = task.of(a[0]).map(|d| faces_of(task, &d)).unwrap_or_default();
    match faces.get(a[1] as u32 as usize) {
        Some(&face) => task.give(cx, a[2], Dw::Font(face)),
        None => {
            let _ = task.write_ptr(cx, a[2], 0);
            Reply::Value(E_INVALIDARG)
        }
    }
}

fn family_get_family_names(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Dw::Family(f)) = task.of(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let name = task.fonts().families[f].name.clone();
    task.give(cx, a[1], Dw::Strings(vec![("en-us".into(), name)]))
}

/// DWRITE_FONT_STYLE: normal, oblique, italic; italic and oblique are matched alike.
fn wants_italic(style: u64) -> bool {
    style as u32 != 0
}

fn family_get_first_matching_font(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Dw::Family(f)) = task.of(a[0]) else { return Reply::Value(E_INVALIDARG) };
    match task.fonts().matching(f, a[1] as u16, a[2] as u16, wants_italic(a[3])) {
        Some(face) => task.give(cx, a[4], Dw::Font(face)),
        None => {
            let _ = task.write_ptr(cx, a[4], 0);
            Reply::Value(DWRITE_E_NOFONT)
        }
    }
}

/// `GetMatchingFonts(weight, stretch, style, &list)`: every face of the family, the nearest first.
fn family_get_matching_fonts(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Dw::Family(f)) = task.of(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let fonts = task.fonts();
    let mut faces = fonts.families[f].faces.clone();
    let (weight, stretch, italic) = (i32::from(a[1] as u16), i32::from(a[2] as u16), wants_italic(a[3]));
    faces.sort_by_key(|&i| {
        let face = &fonts.faces[i];
        ((i32::from(face.stretch) - stretch).abs(), face.italic != italic, (i32::from(face.weight) - weight).abs())
    });
    task.give(cx, a[4], Dw::List(faces))
}

// ---- Fonts ----

fn font_index(task: &Task, this: u64) -> Option<usize> {
    match task.of(this)? {
        Dw::Font(i) | Dw::Face(i) => Some(i),
        _ => None,
    }
}

fn font_get_font_family(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let family = task.fonts().families.iter().position(|f| f.faces.contains(&face)).unwrap_or(0);
    task.give(cx, a[1], Dw::Family(family))
}

fn font_get_weight(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(font_index(task, a[0]).map_or(400, |i| u64::from(task.fonts().faces[i].weight)))
}

fn font_get_stretch(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(font_index(task, a[0]).map_or(5, |i| u64::from(task.fonts().faces[i].stretch)))
}

fn font_get_style(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(font_index(task, a[0]).map_or(0, |i| if task.fonts().faces[i].italic { 2 } else { 0 }))
}

/// Whether its character map is a symbol font's (Windows' symbol encoding, 3/0), as Wingdings' is.
fn is_symbol(task: &Task, face: usize) -> bool {
    let Some(f) = task.face(face) else { return false };
    let Some(cmap) = f.table(*b"cmap") else { return false };
    let n = usize::from(u16::from_be_bytes([cmap.get(2).copied().unwrap_or(0), cmap.get(3).copied().unwrap_or(0)]));
    (0..n).all(|i| cmap.get(4 + 8 * i..8 + 8 * i).is_some_and(|e| e == [0, 3, 0, 0]))
}

fn font_is_symbol_font(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(font_index(task, a[0]).map_or(0, |i| u64::from(is_symbol(task, i))))
}

fn font_get_face_names(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let style = task.fonts().faces[face].style.clone();
    task.give(cx, a[1], Dw::Strings(vec![("en-us".into(), style)]))
}

/// `GetInformationalStrings(id, &strings, &exists)`: the full name (DWRITE_INFORMATIONAL_STRING_FULL_NAME, 15), the
/// PostScript name (16), and the family and face names Win32 and typographic families give (1, 2, 13, 14).
fn font_get_informational_strings(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let f = task.fonts().faces[face].clone();
    let value = match a[1] as u32 {
        1 | 13 => Some(f.family),
        2 | 14 => Some(f.style),
        15 => Some(f.full_name),
        16 => Some(f.id),
        _ => None,
    };
    task.dw_put_u32(cx, a[3], u32::from(value.is_some()));
    match value {
        Some(v) => task.give(cx, a[2], Dw::Strings(vec![("en-us".into(), v)])),
        None => {
            let _ = task.write_ptr(cx, a[2], 0);
            Reply::Value(S_OK)
        }
    }
}

fn font_get_simulations(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

/// DWRITE_FONT_METRICS, 20 bytes.
fn metrics_bytes(face: &Face) -> Vec<u8> {
    let m = &face.metrics;
    let mut b = Vec::new();
    for v in [m.units_per_em, m.ascent, m.descent, m.line_gap as u16, m.cap_height, m.x_height, m.underline_position as u16, m.underline_thickness, m.strikeout_position as u16, m.strikeout_thickness] {
        b.extend(v.to_le_bytes());
    }
    b
}

fn font_get_metrics(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    if let Some(face) = font_index(task, a[0]).and_then(|i| task.face(i)) {
        let b = metrics_bytes(&face);
        task.dw_put(cx, a[1], &b);
    }
    Reply::Value(0)
}

fn font_has_character(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let has = font_index(task, a[0]).and_then(|i| task.face(i)).is_some_and(|f| f.glyph(a[1] as u32) != 0);
    task.dw_put_u32(cx, a[2], u32::from(has));
    Reply::Value(S_OK)
}

fn font_create_font_face(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    if task.face(face).is_none() {
        let _ = task.write_ptr(cx, a[1], 0);
        return Reply::Value(DWRITE_E_FILEFORMAT);
    }
    task.give(cx, a[1], Dw::Face(face))
}

// ---- Localized strings ----

fn strings(task: &Task, this: u64) -> Vec<(String, String)> {
    match task.of(this) {
        Some(Dw::Strings(s)) => s,
        _ => Vec::new(),
    }
}

fn strings_get_count(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(strings(task, a[0]).len() as u64)
}

fn strings_find_locale_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let locale = task.text_arg(cx, a[1], true);
    let found = strings(task, a[0]).iter().position(|(l, _)| l.eq_ignore_ascii_case(&locale));
    task.dw_put_u32(cx, a[2], found.map_or(u32::MAX, |i| i as u32));
    task.dw_put_u32(cx, a[3], u32::from(found.is_some()));
    Reply::Value(S_OK)
}

/// A string's length in characters, or E_INVALIDARG for an index past the end.
fn string_length(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], locale: bool) -> Reply {
    let s = strings(task, a[0]);
    let Some((l, v)) = s.get(a[1] as u32 as usize) else {
        task.dw_put_u32(cx, a[2], 0);
        return Reply::Value(E_INVALIDARG);
    };
    let n = if locale { l } else { v }.encode_utf16().count();
    task.dw_put_u32(cx, a[2], n as u32);
    Reply::Value(S_OK)
}

/// A string into a buffer of `size` characters, with its NUL: E_NOT_SUFFICIENT_BUFFER if it hasn't room.
fn string_copy(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], locale: bool) -> Reply {
    let s = strings(task, a[0]);
    let Some((l, v)) = s.get(a[1] as u32 as usize) else { return Reply::Value(E_INVALIDARG) };
    let units: Vec<u16> = if locale { l } else { v }.encode_utf16().collect();
    if (a[3] as u32 as usize) <= units.len() {
        if a[3] as u32 != 0 {
            task.dw_put(cx, a[2], &[0, 0]);
        }
        return Reply::Value(E_NOT_SUFFICIENT_BUFFER);
    }
    let bytes: Vec<u8> = units.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect();
    task.dw_put(cx, a[2], &bytes);
    Reply::Value(S_OK)
}

fn strings_get_locale_name_length(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    string_length(task, cx, a, true)
}

fn strings_get_locale_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    string_copy(task, cx, a, true)
}

fn strings_get_string_length(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    string_length(task, cx, a, false)
}

fn strings_get_string(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    string_copy(task, cx, a, false)
}

// ---- Font faces ----

/// DWRITE_FONT_FACE_TYPE: CFF (0) for a face with CFF outlines, else TrueType (1).
fn face_get_type(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    let cff = font_index(task, a[0]).and_then(|i| task.face(i)).is_some_and(|f| f.host.tables().iter().any(|t| t == b"CFF " || t == b"CFF2"));
    Reply::Value(if cff { 0 } else { 1 })
}

/// `GetFiles(&count, files)`: its one file; with no array, how many.
fn face_get_files(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    task.dw_put_u32(cx, a[1], 1);
    if a[2] == 0 {
        return Reply::Value(S_OK);
    }
    task.give(cx, a[2], Dw::File(face))
}

fn face_get_simulations(_: &mut Task, _: &mut Cx<'_>, _: &[u64]) -> Reply {
    Reply::Value(0)
}

fn face_is_symbol_font(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    font_is_symbol_font(task, cx, a)
}

fn face_get_metrics(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    font_get_metrics(task, cx, a)
}

fn face_get_glyph_count(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(font_index(task, a[0]).and_then(|i| task.face(i)).map_or(0, |f| u64::from(f.metrics.glyph_count)))
}

/// DWRITE_GLYPH_METRICS for a glyph, in font units: its side bearings, advances and vertical origin.
fn glyph_metrics(face: &Face, glyph: u16) -> [i32; 7] {
    let (advance, lsb) = face.h_metrics(glyph);
    let (v_advance, tsb) = face.v_metrics(glyph);
    let (advance, v_advance) = (i32::from(advance), i32::from(v_advance));
    match face.bounds(glyph) {
        Some((l, b, r, t)) => {
            let (l, b, r, t) = (l.floor() as i32, b.floor() as i32, r.ceil() as i32, t.ceil() as i32);
            let origin = i32::from(tsb) + t;
            let _ = lsb;
            [l, advance, advance - r, i32::from(tsb), v_advance, v_advance - origin + b, origin]
        }
        None => [0, advance, advance, 0, v_advance, v_advance, i32::from(face.metrics.ascent)],
    }
}

fn put_glyph_metrics(task: &mut Task, cx: &mut Cx<'_>, face: &Face, indices_at: u64, count: usize, out: u64, sideways: bool) -> Reply {
    let indices = task.read_u16s(cx, indices_at, count);
    let mut b = Vec::with_capacity(28 * count);
    for g in indices {
        let mut m = glyph_metrics(face, g);
        if sideways {
            // Sideways: the vertical metrics become the horizontal ones.
            m = [m[3], m[4] as u32 as i32, m[5], m[0], m[1], m[2], m[6]];
        }
        for v in m {
            b.extend(v.to_le_bytes());
        }
    }
    if !task.dw_put(cx, out, &b) && count != 0 {
        return Reply::Value(E_INVALIDARG);
    }
    Reply::Value(S_OK)
}

/// `GetDesignGlyphMetrics(&indices, count, &metrics, sideways)`.
fn face_get_design_glyph_metrics(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[0]).and_then(|i| task.face(i)) else { return Reply::Value(E_INVALIDARG) };
    put_glyph_metrics(task, cx, &face, a[1], (a[2] as u32 as usize).min(1 << 16), a[3], a[4] as u32 != 0)
}

/// `GetGdiCompatibleGlyphMetrics(size, ppd, &transform, natural, &indices, count, &metrics, sideways)`: the design
/// metrics, as outlines aren't hinted here.
fn face_get_gdi_compatible_glyph_metrics(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[0]).and_then(|i| task.face(i)) else { return Reply::Value(E_INVALIDARG) };
    let (indices, count, out, sideways) = (task.int_arg(a, 5, 3), task.int_arg(a, 6, 4), task.int_arg(a, 7, 5), task.int_arg(a, 8, 6));
    put_glyph_metrics(task, cx, &face, indices, (count as u32 as usize).min(1 << 16), out, sideways as u32 != 0)
}

/// `GetGdiCompatibleMetrics(size, ppd, &transform, &metrics)`: the design metrics.
fn face_get_gdi_compatible_metrics(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[0]).and_then(|i| task.face(i)) else { return Reply::Value(E_INVALIDARG) };
    let out = task.int_arg(a, 4, 2);
    let b = metrics_bytes(&face);
    task.dw_put(cx, out, &b);
    Reply::Value(S_OK)
}

/// `GetGlyphIndices(&code_points, count, &indices)`: 0 for a character it hasn't.
fn face_get_glyph_indices(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[0]).and_then(|i| task.face(i)) else { return Reply::Value(E_INVALIDARG) };
    let count = (a[2] as u32 as usize).min(1 << 16);
    let points = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], count * 4)).unwrap_or_default();
    let out: Vec<u8> = points.chunks(4).flat_map(|c| face.glyph(u32::from_le_bytes([c[0], c[1], c[2], c[3]])).to_le_bytes()).collect();
    if !task.dw_put(cx, a[3], &out) && count != 0 {
        return Reply::Value(E_INVALIDARG);
    }
    Reply::Value(S_OK)
}

/// `TryGetFontTable(tag, &data, &size, &context, &exists)`: the table in the process's memory, kept there (so
/// ReleaseFontTable has nothing to do); with the tag as OpenType has it, a little-endian number of its four bytes.
fn face_try_get_font_table(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(index) = font_index(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let tag = (a[1] as u32).to_le_bytes();
    let table = task.face(index).and_then(|f| f.table(tag));
    let at = table.as_ref().and_then(|t| task.copy(cx, index, a[1] as u32, t));
    let _ = task.write_ptr(cx, a[2], at.unwrap_or(0));
    task.dw_put_u32(cx, a[3], table.as_ref().map_or(0, |t| t.len() as u32));
    let _ = task.write_ptr(cx, a[4], 0);
    task.dw_put_u32(cx, a[5], u32::from(at.is_some()));
    Reply::Value(S_OK)
}

/// `GetRecommendedRenderingMode(size, ppd, measuring, &params, &mode)`: natural and symmetric, but outlines past
/// 100 pixels, as DirectWrite's own.
fn face_get_recommended_rendering_mode(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (size, ppd) = (task.f32_arg(cx, a, 1, 0), task.f32_arg(cx, a, 2, 1));
    let out = task.int_arg(a, 5, 3);
    let pixels = size * if ppd > 0.0 { ppd } else { 1.0 };
    task.dw_put_u32(cx, out, if pixels > 100.0 { MODE_OUTLINE } else { 5 });
    Reply::Value(S_OK)
}

// ---- Font files, their loader and streams ----

/// `GetReferenceKey(&key, &size)`: the face's number, four bytes, kept with the process's copies.
fn file_get_reference_key(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Dw::File(face)) = task.of(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let key = (face as u32).to_le_bytes();
    let at = task.copy(cx, face, u32::from_le_bytes(*b"key "), &key);
    let _ = task.write_ptr(cx, a[1], at.unwrap_or(0));
    task.dw_put_u32(cx, a[2], 4);
    Reply::Value(if at.is_some() { S_OK } else { E_OUTOFMEMORY })
}

fn file_get_loader(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.give(cx, a[1], Dw::Loader)
}

/// `Analyze(&supported, &file_type, &face_type, &faces)`: an OpenType file (DWRITE_FONT_FILE_TYPE_TRUETYPE or _CFF),
/// with one face.
fn file_analyze(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Dw::File(face)) = task.of(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let cff = task.face(face).is_some_and(|f| f.host.tables().iter().any(|t| t == b"CFF " || t == b"CFF2"));
    task.dw_put_u32(cx, a[1], 1);
    task.dw_put_u32(cx, a[2], if cff { 1 } else { 2 });
    task.dw_put_u32(cx, a[3], if cff { 0 } else { 1 });
    task.dw_put_u32(cx, a[4], 1);
    Reply::Value(S_OK)
}

/// `CreateStreamFromKey(&key, size, &stream)`: a key one of this loader's files gave.
fn loader_create_stream_from_key(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let key = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], 4)).unwrap_or_default();
    let face = if a[2] == 4 && key.len() == 4 { u32::from_le_bytes([key[0], key[1], key[2], key[3]]) as usize } else { usize::MAX };
    if face >= task.fonts().faces.len() || task.face(face).is_none() {
        let _ = task.write_ptr(cx, a[3], 0);
        return Reply::Value(E_INVALIDARG);
    }
    task.give(cx, a[3], Dw::Stream(face))
}

/// `ReadFileFragment(&start, offset, size, &context)`: where the bytes are, in the process's copy of the file.
fn stream_read_file_fragment(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Dw::Stream(face)) = task.of(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let Some(f) = task.face(face) else { return Reply::Value(E_INVALIDARG) };
    let file = f.file();
    let (offset, size) = (a[2], a[3]);
    if offset.checked_add(size).is_none_or(|end| end > file.len() as u64) {
        let _ = task.write_ptr(cx, a[1], 0);
        return Reply::Value(E_INVALIDARG);
    }
    let Some(at) = task.copy(cx, face, 0, &file) else { return Reply::Value(E_OUTOFMEMORY) };
    let _ = task.write_ptr(cx, a[1], at + offset);
    let _ = task.write_ptr(cx, a[4], 0);
    Reply::Value(S_OK)
}

fn stream_get_file_size(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Dw::Stream(face)) = task.of(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let size = task.face(face).map_or(0, |f| f.file().len() as u64);
    task.dw_put(cx, a[1], &size.to_le_bytes());
    Reply::Value(S_OK)
}

/// The file's time: the start of 2020, as the machine's fonts never change while it runs.
fn stream_get_last_write_time(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    task.dw_put(cx, a[1], &132_223_104_000_000_000u64.to_le_bytes());
    Reply::Value(S_OK)
}

// ---- Rendering parameters ----

fn params(task: &Task, this: u64) -> (f32, f32, f32, u32, u32) {
    match task.of(this) {
        Some(Dw::Params { gamma, contrast, clear_type, geometry, mode }) => (gamma, contrast, clear_type, geometry, mode),
        _ => (1.8, 0.5, 1.0, 1, 0),
    }
}

fn params_get_gamma(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = params(task, a[0]).0;
    task.return_f32(cx, v)
}

fn params_get_enhanced_contrast(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = params(task, a[0]).1;
    task.return_f32(cx, v)
}

fn params_get_clear_type_level(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = params(task, a[0]).2;
    task.return_f32(cx, v)
}

fn params_get_pixel_geometry(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(params(task, a[0]).3))
}

fn params_get_rendering_mode(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    Reply::Value(u64::from(params(task, a[0]).4))
}

// ---- Glyph run analysis ----

/// DWRITE_TEXTURE_ALIASED_1x1 and DWRITE_TEXTURE_CLEARTYPE_3x1.
const TEXTURE_ALIASED: u32 = 0;
const TEXTURE_CLEAR_TYPE: u32 = 1;

/// `GetAlphaTextureBounds(type, &rect)`: the run's box for the texture its mode makes, empty for the other.
fn analysis_get_alpha_texture_bounds(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Dw::Analysis { left, top, width, height, aliased, .. }) = task.of(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let kind = a[1] as u32;
    if kind > TEXTURE_CLEAR_TYPE {
        return Reply::Value(E_INVALIDARG);
    }
    let wanted = (kind == TEXTURE_ALIASED) == aliased && width > 0;
    let rect = if wanted { [left, top, left + width as i32, top + height as i32] } else { [0; 4] };
    let b: Vec<u8> = rect.iter().flat_map(|v| v.to_le_bytes()).collect();
    task.dw_put(cx, a[2], &b);
    Reply::Value(S_OK)
}

/// `CreateAlphaTexture(type, &bounds, buffer, size)`: a byte a pixel (0 or 255, aliased) or three alike (grey, as
/// ClearType's three would be), for the bounds asked, the run's pixels where they fall in them.
fn analysis_create_alpha_texture(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(Dw::Analysis { left, top, width, height, alpha, aliased }) = task.of(a[0]) else { return Reply::Value(E_INVALIDARG) };
    let kind = a[1] as u32;
    if kind > TEXTURE_CLEAR_TYPE {
        return Reply::Value(E_INVALIDARG);
    }
    let r = task.mem(cx, false, |t| user::read_vec(&t.space(), a[2], 16)).unwrap_or_default();
    if r.len() < 16 {
        return Reply::Value(E_INVALIDARG);
    }
    let v = |i: usize| i32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]);
    let (l, t, rr, b) = (v(0), v(4), v(8), v(12));
    let (w, h) = ((rr - l).max(0) as usize, (b - t).max(0) as usize);
    let bpp = if kind == TEXTURE_CLEAR_TYPE { 3 } else { 1 };
    if (a[4] as u32 as usize) < w * h * bpp {
        return Reply::Value(E_NOT_SUFFICIENT_BUFFER);
    }
    let matches = (kind == TEXTURE_ALIASED) == aliased;
    let mut out = vec![0u8; w * h * bpp];
    if matches && !alpha.is_empty() {
        for y in 0..h {
            let sy = t + y as i32 - top;
            if sy < 0 || sy as usize >= height {
                continue;
            }
            for x in 0..w {
                let sx = l + x as i32 - left;
                if sx < 0 || sx as usize >= width {
                    continue;
                }
                let c = alpha[sy as usize * width + sx as usize];
                let c = if aliased { if c >= 128 { 255 } else { 0 } } else { c };
                out[(y * w + x) * bpp..(y * w + x + 1) * bpp].fill(c);
            }
        }
    }
    task.dw_put(cx, a[3], &out);
    Reply::Value(S_OK)
}

/// `GetAlphaBlendParams(&params, &gamma, &contrast, &clear_type)`: the parameters' own.
fn analysis_get_alpha_blend_params(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (gamma, contrast, clear_type, _, _) = params(task, a[1]);
    for (at, v) in [(a[2], gamma), (a[3], contrast), (a[4], clear_type)] {
        task.dw_put(cx, at, &v.to_le_bytes());
    }
    Reply::Value(S_OK)
}

// ---- GDI interop ----

/// LOGFONTW: lfHeight, lfWidth, lfEscapement, lfOrientation, lfWeight, then bytes (italic at 20), the face name at 28.
const LOGFONT_SIZE: usize = 92;

/// `CreateFontFromLOGFONT(&logfont, &font)`: its family (or a stand-in), nearest its weight and slant.
fn interop_create_font_from_logfont(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let lf = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], LOGFONT_SIZE)).unwrap_or_default();
    if lf.len() < LOGFONT_SIZE {
        return Reply::Value(E_INVALIDARG);
    }
    let weight = match i32::from_le_bytes([lf[16], lf[17], lf[18], lf[19]]) {
        0 => 400,
        w => w.clamp(1, 999) as u16,
    };
    let units: Vec<u16> = lf[28..92].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
    let name = String::from_utf16_lossy(&units);
    let fonts = task.fonts();
    match fonts.find(&name).and_then(|f| fonts.matching(f, weight, 5, lf[20] != 0)) {
        Some(face) => task.give(cx, a[2], Dw::Font(face)),
        None => {
            let _ = task.write_ptr(cx, a[2], 0);
            Reply::Value(DWRITE_E_NOFONT)
        }
    }
}

/// A LOGFONTW for a face: its family's name, weight and slant, 0 height.
fn logfont_of(task: &Task, face: usize) -> Vec<u8> {
    let f = &task.fonts().faces[face];
    let mut lf = vec![0u8; LOGFONT_SIZE];
    lf[16..20].copy_from_slice(&i32::from(f.weight).to_le_bytes());
    lf[20] = u8::from(f.italic);
    lf[23] = 1; // DEFAULT_CHARSET
    lf[26] = 5; // CLEARTYPE_QUALITY
    for (i, u) in f.family.encode_utf16().take(31).enumerate() {
        lf[28 + 2 * i..30 + 2 * i].copy_from_slice(&u.to_le_bytes());
    }
    lf
}

fn interop_convert_font_to_logfont(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[1]) else { return Reply::Value(E_INVALIDARG) };
    let lf = logfont_of(task, face);
    task.dw_put(cx, a[2], &lf);
    task.dw_put_u32(cx, a[3], 1);
    Reply::Value(S_OK)
}

fn interop_convert_font_face_to_logfont(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(face) = font_index(task, a[1]) else { return Reply::Value(E_INVALIDARG) };
    let lf = logfont_of(task, face);
    task.dw_put(cx, a[2], &lf);
    Reply::Value(S_OK)
}

// ---- Text formats ----

/// `CreateTextFormat(family, collection, weight, style, stretch, size, locale, &format)`.
fn factory_create_text_format(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let family = task.text_arg(cx, a[1], true);
    let size = task.f32_arg(cx, a, 6, 0);
    let (locale_at, out) = (task.int_arg(a, 7, 6), task.int_arg(a, 8, 7));
    let locale = if locale_at == 0 { String::new() } else { task.text_arg(cx, locale_at, true) };
    if !(size > 0.0) {
        let _ = task.write_ptr(cx, out, 0);
        return Reply::Value(E_INVALIDARG);
    }
    let format = Format {
        family,
        weight: a[3] as u16,
        style: a[4] as u32,
        stretch: a[5] as u16,
        size,
        locale,
        alignment: 0,
        paragraph: 0,
        wrapping: 0,
        reading: 0,
        flow: 0,
        tab: size * 4.0,
        trimming: [0; 3],
        spacing: (0, 0.0, 0.0),
    };
    task.give(cx, out, Dw::Format(Box::new(format)))
}

/// Works on a format, or a layout's (which lays itself out again after).
fn with_format<R>(task: &Task, this: u64, f: impl FnOnce(&mut Format) -> R) -> Option<R> {
    let fonts = task.fonts();
    let host = task.k.host.clone();
    task.dw(this, |d| match d {
        Dw::Format(format) => Some(f(format)),
        Dw::Layout(layout) => {
            let r = f(&mut layout.format);
            layout.lay_out(fonts, &host);
            Some(r)
        }
        _ => None,
    })
    .flatten()
}

fn format_of(task: &Task, this: u64) -> Option<Format> {
    match task.of(this)? {
        Dw::Format(f) => Some(*f),
        Dw::Layout(l) => Some(l.format),
        _ => None,
    }
}

/// Sets one of a format's enumerations, if it's one of the values it can be.
fn set_enum(task: &mut Task, a: &[u64], most: u32, field: fn(&mut Format) -> &mut u32) -> Reply {
    let v = a[1] as u32;
    if v > most {
        return Reply::Value(E_INVALIDARG);
    }
    Reply::Value(if with_format(task, a[0], |f| *field(f) = v).is_some() { S_OK } else { E_INVALIDARG })
}

fn format_set_text_alignment(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_enum(task, a, 3, |f| &mut f.alignment)
}

fn format_set_paragraph_alignment(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_enum(task, a, 2, |f| &mut f.paragraph)
}

fn format_set_word_wrapping(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_enum(task, a, 4, |f| &mut f.wrapping)
}

fn format_set_reading_direction(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_enum(task, a, 3, |f| &mut f.reading)
}

fn format_set_flow_direction(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    set_enum(task, a, 3, |f| &mut f.flow)
}

fn format_set_incremental_tab_stop(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let tab = task.f32_arg(cx, a, 1, 0);
    if !(tab > 0.0) {
        return Reply::Value(E_INVALIDARG);
    }
    Reply::Value(if with_format(task, a[0], |f| f.tab = tab).is_some() { S_OK } else { E_INVALIDARG })
}

/// `SetTrimming(&trimming, sign)`: kept; text isn't cut to it yet, and the sign isn't drawn.
fn format_set_trimming(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let t = task.mem(cx, false, |t| user::read_vec(&t.space(), a[1], 12)).unwrap_or_default();
    if t.len() < 12 {
        return Reply::Value(E_INVALIDARG);
    }
    let v = |i: usize| u32::from_le_bytes([t[i], t[i + 1], t[i + 2], t[i + 3]]);
    let trimming = [v(0), v(4), v(8)];
    Reply::Value(if with_format(task, a[0], |f| f.trimming = trimming).is_some() { S_OK } else { E_INVALIDARG })
}

/// `SetLineSpacing(method, spacing, baseline)`.
fn format_set_line_spacing(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let method = a[1] as u32;
    let (spacing, baseline) = (task.f32_arg(cx, a, 2, 0), task.f32_arg(cx, a, 3, 1));
    if method > 2 || (method == 1 && (spacing <= 0.0 || baseline < 0.0)) {
        return Reply::Value(E_INVALIDARG);
    }
    Reply::Value(if with_format(task, a[0], |f| f.spacing = (method, spacing, baseline)).is_some() { S_OK } else { E_INVALIDARG })
}

fn format_get(task: &Task, this: u64, f: fn(&Format) -> u32) -> Reply {
    Reply::Value(format_of(task, this).map_or(0, |format| u64::from(f(&format))))
}

fn format_get_text_alignment(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_get(task, a[0], |f| f.alignment)
}

fn format_get_paragraph_alignment(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_get(task, a[0], |f| f.paragraph)
}

fn format_get_word_wrapping(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_get(task, a[0], |f| f.wrapping)
}

fn format_get_reading_direction(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_get(task, a[0], |f| f.reading)
}

fn format_get_flow_direction(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_get(task, a[0], |f| f.flow)
}

fn format_get_font_weight(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_get(task, a[0], |f| u32::from(f.weight))
}

fn format_get_font_style(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_get(task, a[0], |f| f.style)
}

fn format_get_font_stretch(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_get(task, a[0], |f| u32::from(f.stretch))
}

fn format_get_font_family_name_length(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_get(task, a[0], |f| f.family.encode_utf16().count() as u32)
}

fn format_get_locale_name_length(task: &mut Task, _: &mut Cx<'_>, a: &[u64]) -> Reply {
    format_get(task, a[0], |f| f.locale.encode_utf16().count() as u32)
}

fn format_get_incremental_tab_stop(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = format_of(task, a[0]).map_or(0.0, |f| f.tab);
    task.return_f32(cx, v)
}

fn format_get_font_size(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = format_of(task, a[0]).map_or(0.0, |f| f.size);
    task.return_f32(cx, v)
}

/// `GetTrimming(&trimming, &sign)`: no sign.
fn format_get_trimming(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(f) = format_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let b: Vec<u8> = f.trimming.iter().flat_map(|v| v.to_le_bytes()).collect();
    task.dw_put(cx, a[1], &b);
    let _ = task.write_ptr(cx, a[2], 0);
    Reply::Value(S_OK)
}

fn format_get_line_spacing(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(f) = format_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    task.dw_put_u32(cx, a[1], f.spacing.0);
    task.dw_put(cx, a[2], &f.spacing.1.to_le_bytes());
    task.dw_put(cx, a[3], &f.spacing.2.to_le_bytes());
    Reply::Value(S_OK)
}

/// A name into a buffer of `size` characters with its NUL, or E_NOT_SUFFICIENT_BUFFER.
fn put_name(task: &mut Task, cx: &mut Cx<'_>, at: u64, size: u64, name: &str) -> Reply {
    let units: Vec<u16> = name.encode_utf16().collect();
    if (size as u32 as usize) <= units.len() {
        return Reply::Value(E_NOT_SUFFICIENT_BUFFER);
    }
    let b: Vec<u8> = units.iter().chain([&0]).flat_map(|u| u.to_le_bytes()).collect();
    task.dw_put(cx, at, &b);
    Reply::Value(S_OK)
}

fn format_get_font_family_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(f) = format_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    put_name(task, cx, a[1], a[2], &f.family)
}

fn format_get_locale_name(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(f) = format_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    put_name(task, cx, a[1], a[2], &f.locale)
}

// ---- Text layouts ----

/// DWRITE_WORD_WRAPPING_NO_WRAP.
const NO_WRAP: u32 = 1;

fn is_space(c: u16) -> bool {
    matches!(c, 0x20 | 0x09 | 0xa0 | 0x3000)
}

fn is_newline(c: u16) -> bool {
    matches!(c, 0x0a | 0x0d | 0x0b | 0x0c | 0x85 | 0x2028 | 0x2029)
}

impl Layout {
    /// Its clusters measured in the format's font (the family asked, a stand-in, or else the first there is), and
    /// broken into lines: at newlines, and where the next word wouldn't fit, unless it's not to wrap.
    fn lay_out(&mut self, fonts: &Fonts, host: &Arc<dyn host::Host>) {
        let f = &self.format;
        let family = fonts.find(&f.family).or_else(|| fonts.find("Segoe UI")).or(if fonts.families.is_empty() { None } else { Some(0) });
        let face = family.and_then(|fam| fonts.matching(fam, f.weight, f.stretch, f.style != 0)).and_then(|i| fonts.open(host, i));
        let scale = face.as_ref().map_or(0.0, |face| f.size / f32::from(face.metrics.units_per_em));
        let (ascent, descent, gap) = face.as_ref().map_or((f.size * 0.8, f.size * 0.2, 0.0), |face| {
            let m = &face.metrics;
            (f32::from(m.ascent) * scale, f32::from(m.descent) * scale, f32::from(m.line_gap.max(0)) * scale)
        });
        (self.line_height, self.baseline) = if f.spacing.0 == 1 { (f.spacing.1, f.spacing.2) } else { (ascent + descent + gap, ascent) };
        self.clusters.clear();
        let mut i = 0;
        while i < self.text.len() {
            let c = self.text[i];
            let (code, len) = if (0xd800..0xdc00).contains(&c) && self.text.get(i + 1).is_some_and(|n| (0xdc00..0xe000).contains(n)) {
                (0x10000 + ((u32::from(c) - 0xd800) << 10) + (u32::from(self.text[i + 1]) - 0xdc00), 2)
            } else if c == 0x0d && self.text.get(i + 1) == Some(&0x0a) {
                (0x0d, 2)
            } else {
                (u32::from(c), 1)
            };
            let advance = if is_newline(c) {
                0.0
            } else if c == 0x09 {
                f.tab
            } else {
                face.as_ref().map_or(f.size * 0.5, |face| f32::from(face.h_metrics(face.glyph(code)).0) * scale)
            };
            self.clusters.push((i, len, advance));
            i += len;
        }
        self.lines.clear();
        let wraps = f.wrapping != NO_WRAP && self.max_width > 0.0;
        let mut line = Line::default();
        let mut k = 0;
        while k < self.clusters.len() {
            let c = self.text[self.clusters[k].0];
            if is_newline(c) {
                line.count += 1;
                line.newline = 1;
                self.lines.push(line);
                line = Line { first: k + 1, ..Line::default() };
                k += 1;
                continue;
            }
            // The next word, with the spaces after it.
            let mut end = k;
            let mut word = 0.0;
            while end < self.clusters.len() && !is_space(self.text[self.clusters[end].0]) && !is_newline(self.text[self.clusters[end].0]) {
                word += self.clusters[end].2;
                end += 1;
            }
            let mut spaces = 0.0;
            let mut space_count = 0;
            while end < self.clusters.len() && is_space(self.text[self.clusters[end].0]) {
                spaces += self.clusters[end].2;
                space_count += 1;
                end += 1;
            }
            if wraps && line.count > 0 && line.width_ws + word > self.max_width {
                self.lines.push(line);
                line = Line { first: k, ..Line::default() };
            }
            line.count += end - k;
            line.width = line.width_ws + word;
            line.width_ws = line.width + spaces;
            line.trailing = space_count;
            k = end;
        }
        if line.count > 0 || self.lines.is_empty() || self.lines.last().is_some_and(|l| l.newline > 0) {
            self.lines.push(line);
        }
    }

    /// Where a line starts across the layout's box, as the text is aligned.
    fn line_left(&self, line: &Line) -> f32 {
        let room = (self.max_width - line.width).max(0.0);
        match self.format.alignment {
            1 => room,
            2 => room / 2.0,
            _ => 0.0,
        }
    }

    /// Where the text starts down the box, as the paragraph's aligned.
    fn top(&self) -> f32 {
        let room = (self.max_height - self.lines.len() as f32 * self.line_height).max(0.0);
        match self.format.paragraph {
            1 => room,
            2 => room / 2.0,
            _ => 0.0,
        }
    }

    /// The line a cluster's on, and the cluster's x in the box.
    fn locate(&self, cluster: usize) -> (usize, f32) {
        let n = self.lines.iter().position(|l| cluster < l.first + l.count).unwrap_or(self.lines.len().saturating_sub(1));
        let line = self.lines.get(n).copied().unwrap_or_default();
        let x = self.line_left(&line) + self.clusters[line.first.min(cluster)..cluster].iter().map(|c| c.2).sum::<f32>();
        (n, x)
    }

    /// The cluster a text position is in (the one after the last for the end).
    fn cluster_at(&self, position: usize) -> usize {
        self.clusters.iter().position(|c| position < c.0 + c.1).unwrap_or(self.clusters.len())
    }
}

/// `CreateTextLayout(text, length, format, max_width, max_height, &layout)`.
fn factory_create_text_layout(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let (max_width, max_height) = (task.f32_arg(cx, a, 4, 0), task.f32_arg(cx, a, 5, 1));
    let out = task.int_arg(a, 6, 4);
    let Some(format) = format_of(task, a[3]) else {
        let _ = task.write_ptr(cx, out, 0);
        return Reply::Value(E_INVALIDARG);
    };
    let n = (a[2] as u32 as usize).min(1 << 20);
    let text = task.read_u16s(cx, a[1], n);
    if text.len() != n || max_width < 0.0 || max_height < 0.0 {
        let _ = task.write_ptr(cx, out, 0);
        return Reply::Value(E_INVALIDARG);
    }
    let mut layout = Layout { format, text, max_width, max_height, lines: Vec::new(), clusters: Vec::new(), line_height: 0.0, baseline: 0.0 };
    layout.lay_out(task.fonts(), &task.k.host);
    task.give(cx, out, Dw::Layout(Box::new(layout)))
}

fn layout_of(task: &Task, this: u64) -> Option<Layout> {
    match task.of(this)? {
        Dw::Layout(l) => Some(*l),
        _ => None,
    }
}

fn layout_set(task: &mut Task, cx: &mut Cx<'_>, a: &[u64], width: bool) -> Reply {
    let v = task.f32_arg(cx, a, 1, 0);
    if v < 0.0 {
        return Reply::Value(E_INVALIDARG);
    }
    let fonts = task.fonts();
    let host = task.k.host.clone();
    let done = task.dw(a[0], |d| match d {
        Dw::Layout(l) => {
            if width {
                l.max_width = v;
            } else {
                l.max_height = v;
            }
            l.lay_out(fonts, &host);
            true
        }
        _ => false,
    });
    Reply::Value(if done == Some(true) { S_OK } else { E_INVALIDARG })
}

fn layout_set_max_width(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    layout_set(task, cx, a, true)
}

fn layout_set_max_height(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    layout_set(task, cx, a, false)
}

fn layout_get_max_width(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = layout_of(task, a[0]).map_or(0.0, |l| l.max_width);
    task.return_f32(cx, v)
}

fn layout_get_max_height(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let v = layout_of(task, a[0]).map_or(0.0, |l| l.max_height);
    task.return_f32(cx, v)
}

fn f32s(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// `GetMetrics(&metrics)`: DWRITE_TEXT_METRICS.
fn layout_get_metrics(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(l) = layout_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let width = l.lines.iter().map(|x| x.width).fold(0.0, f32::max);
    let width_ws = l.lines.iter().map(|x| x.width_ws).fold(0.0, f32::max);
    let left = l.lines.iter().map(|x| l.line_left(x)).fold(f32::MAX, f32::min);
    let left = if left == f32::MAX { 0.0 } else { left };
    let height = l.lines.len() as f32 * l.line_height;
    let mut b = f32s(&[left, l.top(), width, width_ws, height, l.max_width, l.max_height]);
    b.extend(1u32.to_le_bytes());
    b.extend((l.lines.len() as u32).to_le_bytes());
    task.dw_put(cx, a[1], &b);
    Reply::Value(S_OK)
}

/// `GetLineMetrics(&metrics, room, &count)`: DWRITE_LINE_METRICS for each line, by text positions.
fn layout_get_line_metrics(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(l) = layout_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    task.dw_put_u32(cx, a[3], l.lines.len() as u32);
    if (a[2] as u32 as usize) < l.lines.len() {
        return Reply::Value(E_NOT_SUFFICIENT_BUFFER);
    }
    let units = |first: usize, count: usize| l.clusters[first..first + count].iter().map(|c| c.1).sum::<usize>() as u32;
    let mut b = Vec::new();
    for line in &l.lines {
        b.extend(units(line.first, line.count).to_le_bytes());
        b.extend(units(line.first + line.count - line.trailing - line.newline, line.trailing + line.newline).to_le_bytes());
        b.extend(units(line.first + line.count - line.newline, line.newline).to_le_bytes());
        b.extend(f32s(&[l.line_height, l.baseline]));
        b.extend(0u32.to_le_bytes());
    }
    task.dw_put(cx, a[1], &b);
    Reply::Value(S_OK)
}

/// `GetClusterMetrics(&metrics, room, &count)`: DWRITE_CLUSTER_METRICS: width, length, and whether a line can break
/// after it, it's whitespace, or a newline.
fn layout_get_cluster_metrics(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(l) = layout_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    task.dw_put_u32(cx, a[3], l.clusters.len() as u32);
    if (a[2] as u32 as usize) < l.clusters.len() {
        return Reply::Value(E_NOT_SUFFICIENT_BUFFER);
    }
    let mut b = Vec::with_capacity(8 * l.clusters.len());
    for &(at, len, width) in &l.clusters {
        let c = l.text[at];
        let flags = u16::from(is_space(c) || is_newline(c)) | u16::from(is_space(c)) << 1 | u16::from(is_newline(c)) << 2;
        b.extend(width.to_le_bytes());
        b.extend((len as u16).to_le_bytes());
        b.extend(flags.to_le_bytes());
    }
    task.dw_put(cx, a[1], &b);
    Reply::Value(S_OK)
}

/// `GetOverhangMetrics(&overhangs)`: how far the text goes past the box each way (negative inside it); outlines'
/// own overhangs aren't counted.
fn layout_get_overhang_metrics(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(l) = layout_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let width = l.lines.iter().map(|x| x.width).fold(0.0, f32::max);
    let left = l.lines.iter().map(|x| l.line_left(x)).fold(f32::MAX, f32::min);
    let left = if left == f32::MAX { 0.0 } else { left };
    let height = l.lines.len() as f32 * l.line_height;
    let b = f32s(&[-left, -l.top(), left + width - l.max_width, l.top() + height - l.max_height]);
    task.dw_put(cx, a[1], &b);
    Reply::Value(S_OK)
}

/// `DetermineMinWidth(&width)`: the widest word.
fn layout_determine_min_width(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(l) = layout_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let mut widest = 0f32;
    let mut word = 0f32;
    for &(at, _, w) in &l.clusters {
        if is_space(l.text[at]) || is_newline(l.text[at]) {
            word = 0.0;
        } else {
            word += w;
            widest = widest.max(word);
        }
    }
    task.dw_put(cx, a[1], &widest.to_le_bytes());
    Reply::Value(S_OK)
}

/// DWRITE_HIT_TEST_METRICS for clusters `first..first + count` of a line, at `x`.
fn hit_metrics(l: &Layout, first: usize, count: usize, x: f32, line: usize) -> Vec<u8> {
    let position = l.clusters.get(first).map_or(l.text.len(), |c| c.0) as u32;
    let length: usize = l.clusters.get(first..first + count).map_or(0, |c| c.iter().map(|c| c.1).sum());
    let width: f32 = l.clusters.get(first..first + count).map_or(0.0, |c| c.iter().map(|c| c.2).sum());
    let mut b = Vec::with_capacity(36);
    b.extend(position.to_le_bytes());
    b.extend((length as u32).to_le_bytes());
    b.extend(f32s(&[x, l.top() + line as f32 * l.line_height, width, l.line_height]));
    b.extend(0u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b
}

/// `HitTestPoint(x, y, &trailing, &inside, &metrics)`.
fn layout_hit_test_point(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(l) = layout_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let (x, y) = (task.f32_arg(cx, a, 1, 0), task.f32_arg(cx, a, 2, 1));
    let (trailing_at, inside_at, out) = (task.int_arg(a, 3, 1), task.int_arg(a, 4, 2), task.int_arg(a, 5, 3));
    let top = l.top();
    let n = (((y - top) / l.line_height.max(1e-6)).floor().max(0.0) as usize).min(l.lines.len().saturating_sub(1));
    let line = l.lines.get(n).copied().unwrap_or_default();
    let mut at = l.line_left(&line);
    let mut hit = line.first + line.count.saturating_sub(line.newline).saturating_sub(1).min(line.count);
    let mut trailing = true;
    for k in line.first..line.first + line.count - line.newline {
        let w = l.clusters[k].2;
        if x < at + w {
            hit = k;
            trailing = x >= at + w / 2.0;
            break;
        }
        at += w;
    }
    let inside = y >= top && y < top + l.lines.len() as f32 * l.line_height && x >= l.line_left(&line) && x < l.line_left(&line) + line.width_ws;
    let hit = hit.min(l.clusters.len().saturating_sub(1));
    let (_, hx) = if l.clusters.is_empty() { (0, 0.0) } else { l.locate(hit) };
    task.dw_put_u32(cx, trailing_at, u32::from(trailing && !l.clusters.is_empty()));
    task.dw_put_u32(cx, inside_at, u32::from(inside));
    let b = hit_metrics(&l, hit, usize::from(!l.clusters.is_empty()), hx, n);
    task.dw_put(cx, out, &b);
    Reply::Value(S_OK)
}

/// `HitTestTextPosition(position, trailing, &x, &y, &metrics)`.
fn layout_hit_test_text_position(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(l) = layout_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let cluster = l.cluster_at(a[1] as u32 as usize);
    let (line, mut x) = if cluster < l.clusters.len() { l.locate(cluster) } else { l.locate(l.clusters.len().saturating_sub(1)) };
    let past_end = cluster >= l.clusters.len() && !l.clusters.is_empty();
    if past_end {
        x += l.clusters.last().map_or(0.0, |c| c.2);
    }
    let base = x;
    if a[2] as u32 != 0 && cluster < l.clusters.len() {
        x += l.clusters[cluster].2;
    }
    task.dw_put(cx, a[3], &x.to_le_bytes());
    task.dw_put(cx, a[4], &(l.top() + line as f32 * l.line_height).to_le_bytes());
    let b = hit_metrics(&l, cluster.min(l.clusters.len()), usize::from(cluster < l.clusters.len()), base, line);
    task.dw_put(cx, a[5], &b);
    Reply::Value(S_OK)
}

/// `HitTestTextRange(position, length, x, y, &metrics, room, &count)`: a rectangle for each line the range is on.
fn layout_hit_test_text_range(task: &mut Task, cx: &mut Cx<'_>, a: &[u64]) -> Reply {
    let Some(l) = layout_of(task, a[0]) else { return Reply::Value(E_INVALIDARG) };
    let (ox, oy) = (task.f32_arg(cx, a, 3, 0), task.f32_arg(cx, a, 4, 1));
    let (out, room, count_at) = (task.int_arg(a, 5, 3), task.int_arg(a, 6, 4), task.int_arg(a, 7, 5));
    let (start, len) = (a[1] as u32 as usize, a[2] as u32 as usize);
    let (first, last) = (l.cluster_at(start), l.cluster_at(start + len));
    let mut rects = Vec::new();
    for (n, line) in l.lines.iter().enumerate() {
        let (s, e) = (first.max(line.first), last.min(line.first + line.count));
        if s >= e {
            continue;
        }
        let x = l.line_left(line) + l.clusters[line.first..s].iter().map(|c| c.2).sum::<f32>();
        let mut m = hit_metrics(&l, s, e - s, x + ox, n);
        let y = f32::from_le_bytes(m[12..16].try_into().unwrap_or_default()) + oy;
        m[12..16].copy_from_slice(&y.to_le_bytes());
        rects.push(m);
    }
    task.dw_put_u32(cx, count_at, rects.len() as u32);
    if (room as u32 as usize) < rects.len() {
        return Reply::Value(E_NOT_SUFFICIENT_BUFFER);
    }
    let b: Vec<u8> = rects.concat();
    task.dw_put(cx, out, &b);
    Reply::Value(S_OK)
}
