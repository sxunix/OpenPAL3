# 菜单页界面粒子（`gamemainui/uiparticle.cpp` 的 `UIParticle`）

移植：逻辑 `shared/src/openpal3/ui_particle.rs`，各页的几组 `yaobow/src/openpal3/particles.rs`、
`sword_fx.rs`。画法 `UIRenderPar`：`ui_par_tex_color.gbf`，SRCALPHA/ONE **加法**，顶点色 = 粒子色 + alpha，
贴图（`Par_Effect\spark/star/flare.tga`，24 位无 alpha）32×32 以粒子为中心。

- 每次 `Render(type)` 先画后走一步（`RISE_HELIX` 后画），步长按帧；移植端每秒 60 步 [近似]。
- `Set_Color` 的 int 截成字节：`252 + rand%10` 会绕回 0..5，所以仙术页施法的火花多半偏红，照留。
- `Set_Position` 以 (x, y) 为上边中点撑开 `Set_AreaSize` 的范围；`RISE` / `RISE_HELIX` 从范围底边往上走，
  到顶就灭；alpha 每步减 `fade`（环绕是 `fade × speed`），减到 0 灭。
- 菜单页用到的几组：
  - **工具栏**（八页都有）：鼠标停在能点的按钮上（`UIWM_MOUSEON`）→ 按钮缩进左 35 / 右 36 / 上下 5 的框，
    16 个红火花 (250,66,58)+rand 顺时针 + 16 个同样的逆时针（`PAR_SURROUND_CW/CCW`），速 1.2、淡 1，约 3 秒
    淡没、不补；移开（`UIWM_MOUSEOFF`）就停。当前页、禁用的按钮不出（禁用按钮不发 MOUSEON）。
  - **头像**：仙术页施法（8 个，色 (219,36,252)，alpha 255，淡 2，速 0.5）、道具页用道具（12 个，色按道具加
    精 (168,168,28) / 气 (88,168,28) / 神 (28,88,168) / 其他 (138,84,224)，alpha 120..254，淡 1，速 0.8）——
    `PAR_RISE` 一组 + 同参数拷贝按 `PAR_RAINBOW` 走一组；范围是头像 65×86。全体道具在**施用之后**才看谁还用
    得上，满了的不冒（照留）。
  - **合成**：等结果 5 秒里 (88,206) 80×80 的 98 个 `PAR_RISE_HELIX`，灭了从左半边随机 x 再放。
  - **御剑页**：见 `sword_page_layout.md` §13。
