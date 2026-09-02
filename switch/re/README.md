# PAL3.exe 逆向工具与笔记

移植要复刻原版行为时，资源包给不出的细节（布局坐标、公式、状态机）从
原版 `PAL3.exe`（v1.4）里逆出来。这里只放脚本和笔记，**不放 exe、dll、
脱壳产物或任何游戏数据**。

## 流程
1. `PAL3_DIR=<游戏目录> python unpack_pal3.py out/PAL3_unpacked.exe`
   —— 用 unicorn 模拟 `pal3.dll` 的加载器导出，静态还原被 StarForce 抽走的
   代码与 IAT，得到可直接分析的 PE（依赖 `pefile unicorn`）。
2. Ghidra headless：
   `analyzeHeadless <proj> pal3 -import PAL3_unpacked.exe -scriptPath . -postScript ExportAll.java <outdir>`
   —— 导出 `decompiled.c`（全部函数）、`functions.txt`、`strings_xref.txt`。
3. 反编译丢了 thiscall 的 `this`（矩形/字号等 setter 看不出目标控件）时，
   用 `python x86dis.py PAL3_unpacked.exe <hexVA> <hexSize>`（capstone）读
   `lea ecx,[edi+0x…]` 前缀。

## 笔记
- `notes/gdb_database.md` —— `cbdata\pal3_softstar.gdb` 四张表（战斗角色 / 仙术 /
  物品 / 合击技）的结构与出处。解析器在 `yaobow/fileformats/src/pal3/gdb.rs`，
  字段布局移植自 Pal3.Unity（GPL-3.0，已署名），记录长度用 exe 独立核对。
- `notes/state_page_layout.md` —— 状态页（GameMainUI/State）全部矩形、
  工具栏顺序/间距、控件渲染模型（贴图原生尺寸贴左上角、16px 字体 + 8px 内边距）、
  进度条内边距、角色结构偏移、AssisLang 标签索引。
  已落实到 `yaobow/yaobow/scripts/openpal3/state_menu.p7`。
- `notes/item_page_layout.md` —— 道具页（GameMainUI/Prop）全部矩形：六个分类页签
  及其 → ItemType 过滤映射、12 行列表网格与滚动条、说明面板、三个排序按钮、
  顶部队伍条；并指出状态页笔记里 TimeMoney 的 x 坐标应为 596 而非 579。
- `notes/equip_page_layout.md` —— 装备页（GameMainUI/Equip）：六个装备槽（武器 +
  帽衣鞋 + 饰品×2）与槽图标、五个分类页签 → 过滤类映射、属性比较区、
  五名角色的面板与 SelRole 按钮，以及穿装备函数 `FUN_00469eb0` 的语义
  （类型/可用角色校验、旧装备退回背包、角色结构 +0x354 的装备数组）。
