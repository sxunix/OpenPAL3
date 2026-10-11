# 原生界面的混合方式（gfxscript 里的几张 .gbf）

> 这一份后来和 `basedata.cpk` 里的源码 `data/pal3/uibase.cpp` 对过，全部吻合。
> 下表最后一列补上了源码里的名字，见 [`src_from_cpk.md`](src_from_cpk.md)。

界面四边形的渲染效果在 `UIInitSystem`（`uibase.cpp:31`，exe 里 `FUN_0053fec0`，
0x53ff00 一带）一次建好，存在一排全局里：

| 全局 | 源码里的名字 | 脚本 | 混合 | 颜色 |
| --- | --- | --- | --- | --- |
| `DAT_01895048` | `UI_EFFECT` | （`NewTrivalEffect`） | — | — |
| `DAT_0189504c` | `UI_EFFECT_TL` | `ui_TL.gbf` | SRCALPHA / INVSRCALPHA | 贴图 × 顶点色 |
| `DAT_01895050` | `UI_EFFECT_NOTEX_TL` | `ui_notex_TL.gbf` | SRCALPHA / INVSRCALPHA | 只有顶点色 |
| `DAT_01895068` | `UI_EFFECT_NOTEX2` | `ui_notex.gbf` | SRCALPHA / INVSRCALPHA | 只有顶点色，**开 Z 测试** |
| `DAT_01895054` | `UI_EFFECT_TEX_VCOLOR` | `ui_tex_vcolor.gbf` | SRCALPHA / INVSRCALPHA | 贴图 **＋** 顶点色（ColorOp=ADD） |
| `DAT_01895058` | `UI_EFFECT_TEX_COLOR` | `ui_tex_color.gbf` | **SRCALPHA / ONE** | 贴图 × 顶点色 |
| `DAT_0189505c` | `UI_EFFECT_TEX_PARTICLE` | `ui_par_tex_color.gbf` | **SRCALPHA / ONE** | 贴图 × 顶点色 |
| `DAT_01895060` | `UI_EFFECT_TEX_COLOR_ALPHA` | `ui_alpha.gbf` | SRCALPHA / INVSRCALPHA | 贴图 × 顶点色，不写 Filter（双线性） |
| `DAT_01895064` | `UI_EFFECT_ADD_COLOR` | `ui_AddColor.gbf` | **ONE / ONE** | 只有顶点色 |
| `DAT_0189506c` | `EFFECT_LINE` | `geom_notex_line.gbf` | — | 画线 |

脚本本身在包里：`/basedata/basedata/gfxscript/*.gbf`，是文本，直接 `cat` 就能看。
`ui_tex_color.gbf` 的内容：

```
FVF = XYZRHW|DIFFUSE|TEX1;  ZEnable = FALSE;  ZWriteEnable = FALSE;
AlphaTestEnable = TRUE;  AlphaBlendEnable = TRUE;
SrcBlend = SRCALPHA;  DestBlend = ONE;
ColorOp[0] = MODULATE; ColorArg1[0] = TEXTURE; ColorArg2[0] = DIFFUSE;
AlphaOp[0] = MODULATE; AlphaArg1[0] = TEXTURE; AlphaArg2[0] = DIFFUSE;
MinFilter[0] = POINT; MagFilter[0] = POINT; AddressU/V[0] = CLAMP;
```

也就是 `dst += 贴图.rgb·色.rgb · 贴图.a·色.a`——**只加不盖**。

## 哪个画图函数用哪张

| 函数 | 效果 | 调用点 |
| --- | --- | --- |
| `FUN_00540070` | `ui_notex_TL` | 纯色块，很多 |
| `FUN_00540210` / `FUN_00540440` / `FUN_00540650` | `ui_TL`（`param_7` 可切 `ui_tex_vcolor`） | 控件贴图的主力 |
| `FUN_005407a0` | `ui_par_tex_color` | 粒子 |
| `FUN_005408e0` | `ui_tex_vcolor` | `0x43b6f1` |
| `FUN_00540a20` | **`ui_tex_color`（加法）** | `0x4429fa` |
| `FUN_00540b20` | `ui_TL` | `0x443439`、`0x4434a0` |
| `FUN_00540c60` | **`ui_tex_color`（加法）** | 只有 `0x43eb66`：**列表选中条** |
| `FUN_00540d20` | **`ui_tex_color`（加法）** | `0x45995d`、`0x47c2bd`、`0x4a487d`、`0x52365d`、`0x527d2d`、`0x529cdb`：界面背后那层场景截图 |
| `FUN_00540dc0` | `ui_alpha`（可给 uv 子矩形） | `0x44035f`、`0x44039e`、`0x44040d`（进度条）、`0x444b46`、`0x444b82` |

源码里对应的名字：`FUN_00540070` = `UIRenderQuad(l,t,r,b,color)`（纯色，`uibase.cpp:69`）、
`FUN_00540c60` = `UIRenderQuadList`（`:533`）、`FUN_005407a0` = `UIRenderPar`（`:329`）、
`FUN_00540dc0` = `UIRenderQuadAlpha`（`:689`）、`FUN_00540a20` = `UIRenderQuadA`（`:561`）。
还有一个带 `mode` 参数的 `UIRenderQuad`（`:264`）：mode 0 走 `ui_tex_color`（加法）、
**mode 1 走 `ui_tex_vcolor`（贴图 + 顶点色）**——大地图目的地悬停就是这一支。

## uv 的朝向

`gbUIQuad` 的头四个 float 是 (左, **下**, 右, **上**)，uv 的 (0,0) 配的是左下角。
贴图上传不翻转，TGA 又是左下角原点（image descriptor 第 5 位为 0），所以
「v = 0」= 文件第 0 行 = 画面最下面一行，两头正好对上，图是正的。

这条和版面常量那条规矩是一回事：**控件矩形的宽高 = 贴图里那块画面的宽高，画面贴在
2 的幂画布的左下角**，所以取 v 要取 `[0, h/贴图高]`（从下往上数）。用 Rust 的
`image` 解出来的是自上而下的图，同一件事写成 `v ∈ [1 − h/高, 1]`，
`encamp::sprite_uv` 就是这么写的。

`FUN_00540c60` 的 u 不裁，固定 `[0, 1]`——**整张贴图横向拉满条宽**。
三处用到它的列表里，当铺的格子正好 256 宽（和 `ListBoxSelPic` 同宽），炼妖
（333）和对话选择框（约 390）都比贴图宽，`sprite_uv` 的 `.min(1.)` 夹完也是
`[0, 1]`，所以移植端这一侧恰好和原版一致。

## 移植端怎么办

imgui 后端整帧只有一种混合状态（普通 alpha），没法逐条命令换。选中条那一处
改成在装贴图的时候就把「该加上去的那一份」折进 alpha：
`A = max(c)`、`RGB = c / A`，见 `openpal3::ui_tex::bake_additive`。
误差是 `dst·A`，两张条的 A 峰值分别是 45/255 和 99/255。

还没做的：界面背后那层场景截图（六处），移植端根本没画。
