// The "ink" look of the Esc menu (ui/menu.cpp, ui/option_rows.cpp): a dark translucent panel, vivid paint colours,
// chunky pills. Palette and small helpers shared by both files. Original work: the colours and shapes follow the
// launcher's own street-art identity (launcher/src/styles/ink.css), nothing comes from a game.
#pragma once
#include <math.h>

#include "imgui.h"

namespace ink {
extern bool on;           // the ink look is in use this frame (other brands keep the upstream look)
extern ImFont *display;   // the heavy display face (JetBrains Mono ExtraBold), or null
extern bool anim;         // short animations are on (TOTK_MENU_ANIM)

// launcher tokens: --bg, --surface, --text*, --mint, --magenta, --lemon, --orange, --blue
constexpr ImU32 kPanel = IM_COL32(18, 13, 44, 236);
constexpr ImU32 kRow = IM_COL32(30, 23, 70, 232);
constexpr ImU32 kRowHi = IM_COL32(48, 38, 104, 244);
constexpr ImU32 kWell = IM_COL32(10, 7, 30, 235);      // the inside of a segmented pill or a track
constexpr ImU32 kLine = IM_COL32(255, 255, 255, 30);
constexpr ImU32 kText = IM_COL32(255, 253, 246, 255);   // 17:1 on kPanel
constexpr ImU32 kText2 = IM_COL32(211, 205, 240, 255);  // 11:1
constexpr ImU32 kText3 = IM_COL32(169, 162, 211, 255);  // 6.6:1
constexpr ImU32 kMint = IM_COL32(31, 242, 192, 255);
constexpr ImU32 kMagenta = IM_COL32(255, 49, 144, 255);
constexpr ImU32 kLemon = IM_COL32(255, 226, 61, 255);
constexpr ImU32 kOrange = IM_COL32(255, 123, 36, 255);
constexpr ImU32 kBlue = IM_COL32(79, 166, 255, 255);
constexpr ImU32 kInk = IM_COL32(20, 12, 44, 255);       // text on a bright fill: 9:1 on mint, 6:1 on magenta

inline ImU32 mix(ImU32 a, ImU32 b, float t) {
    t = t < 0 ? 0 : t > 1 ? 1 : t;
    ImVec4 x = ImGui::ColorConvertU32ToFloat4(a), y = ImGui::ColorConvertU32ToFloat4(b);
    return ImGui::ColorConvertFloat4ToU32(ImVec4(x.x + (y.x - x.x) * t, x.y + (y.y - x.y) * t, x.z + (y.z - x.z) * t,
                                                 x.w + (y.w - x.w) * t));
}
inline ImU32 with_alpha(ImU32 c, float a) {
    a = a < 0 ? 0 : a > 1 ? 1 : a;
    return (c & 0x00FFFFFFu) | ((ImU32)(a * ((c >> 24) & 255)) << 24);
}
// Moves cur towards target; at once when animations are off.
inline float approach(float cur, float target, float dt, float speed) {
    if (!anim) return target;
    float k = 1.0f - expf(-speed * dt);
    return cur + (target - cur) * k;
}
}  // namespace ink
