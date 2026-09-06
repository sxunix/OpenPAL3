# 买卖（SCE 100 `UIShowDealerMenu`）

买卖系统的入口、数据文件格式、价格与还价规则。地址都是 PAL3 v1.4 未加壳
`PAL3_unpacked.exe`。

## 1. 入口

`LAB_00436c00`（opcode 100，一个字符串参数 = 数据文件路径）：

```text
FUN_00457490(ecx = 0x8acd40, 文件名)   // 解析 DealScript 文本
FUN_00457f00(ecx = 0x8acd40)           // 把商品的物品 id 换成数据库指针
FUN_0044f610(2)                        // 进入模式 2 = 买卖
new 一个等待任务 FUN_00434ca0(2) → FUN_00434c40  // 脚本挂起到模式结束
```

`FUN_00457490` 开头 `lstrcmpA(shop + 0xc4c34, 文件名)`：**同一个文件不会重复
解析**，只有换了店才清空重来。

模式 2 的界面对象在 `FUN_0044f8d0` 的 `case 2` 建出来（`FUN_004590d0`
构造，`FUN_004593c0` 布局），随后：

```text
FUN_00459970(shop.ST)         // 按店铺类型摆列表框
FUN_00459f30(&shop.ST)        // 把 0x1118 dword（0x4460 字节）的店铺数据
                              // 整块拷进界面对象 +0x178
```

所以界面里读的偏移都是「`0x178 +（字段在店铺结构里的位置 − 0xc4d34）`」。
下面的偏移一律按**界面对象**记，`+0x178` 就是 `ST`。

## 2. DealScript 文件格式

`basedata/DealScript/{AnyShop,DragShop,Rest,WeaponShop}/*.txt`，GBK，共 49 个。
分词器是 `FUN_004318b0`（整文件读进内存）+ `FUN_004319f0` 设分隔符 +
`FUN_00431a60` 取下一个词；取值用 `FUN_00431b60` 读 `$…&` 之间那一段。
`;` 起注释。关键字表在 `0x5b1a50`（每项 30 字节）。

结构：

```text
Begin,<段名>
  ST $2&            店铺类型：0 药店 · 1 武器店 · 2 杂货店 · 3 商人
  DK $1&            有没有还价系统：0 无 · 1 有（>1 一律按 1）
  M_ID $1411&       商人的角色 id
  M_Name $阿郁&
  MC $0&            性格：0 和善 1 暴躁 2 敦厚 3 软弱 4 谨慎 5 喜怒无常
  Patience $10&     经营者忍耐度
  BeginTalk / EntryBuy / EntrySale / NormalAcptTalk / NormalCnslTalk /
  NoEnoughMoney / Leave / HarshTalkbuy      各一句台词

  FIVE … #         五折档
  SIX … #          六折
  SEVEN … #        七折
  EIGHT … #        八折
  NINE … #         九折
      AcptQuantum $0.1&     接受概率
      AcptTalk1/2           接受台词（两句随机取一）
      RfusQuantum $0.7&     拒绝概率
      RfusTalk1/2
      HarshQuantum $0.2&    严词拒绝概率（**代码里不读**，见 §4）
      HarshTalk1/2
      ForceAcptTalkto1/2    忍耐度用光时商人的话
      ForceAcptTalkback     景天的回话

  Equip … #        卖装备的清单，每行 `Item_ID $80001&`
  Prop … #         卖道具的清单
end
```

台词里的 `\n` 是换行。`Equip`/`Prop` 两段各自最多 400 项
（`FUN_00457490` 清的两块分别是 400 和 100 个 dword）。

`FUN_00457f00` 在解析后把两段的物品 id 逐个换成数据库记录指针
（`FUN_00479130(id)`），换不到的留空。

## 3. 买：单价、总价与库存

- 单价就是物品数据库的 `price`（`GameItemInfo +0x260`）。
- 界面把「这一单要付多少」放在 `+0x160`。
- 每种商品**最多能买到 99 个**：`FUN_0045b7a0` 建列表时对每一项查一次背包
  （`FUN_0040dcf0`），可买数 = `99 − 已持有数`，背包里没有就是 99。
- `DK == 0`（不还价）时界面盖上 `UI\Assistant\Communal\UnDicker.tga`。

## 4. 还价（`FUN_00461c50` → `FUN_00460a90`）

玩家挑一个折扣档 `tier`（0 = 五折 … 4 = 九折）。

先看钱够不够打完折的价：`总价 × (5 + tier) / 10`（**整数除法**），不够就直接
弹 `NoEnoughMoney`、把状态置 1 并结束。

够了才掷骰：

```text
p_acpt = AcptQuantum[tier]     // 负数 → 现掷一个 rand15()%100 * 0.01 当概率
p_rfus = RfusQuantum[tier]     // 同上
if 掷中 p_acpt        → 状态 1 接受
else if 掷中 p_rfus   → 状态 0 拒绝，拒绝次数 +1
                        次数 ≥ Patience → 状态 2 强制接受
else                  → 状态 3 严词拒绝
```

掷骰函数 `FUN_00460a00(p)`：

```text
n = ftol(p × 100)                     // 截断
if n <= 0: false
r = (seed = seed*0x343fd + 0x269ec3) >> 16 & 0x3ff    // 0..1023
return r < round(n × 10.24)
```

——和逃跑的掷法同一套（`combat_formulas.md` §逃跑）。`HarshQuantum` 只是数据
文件里的注释性字段，代码把「两次都没掷中」当作严词拒绝，**没有读它**。

结算（`FUN_00460a90(tier, 应用折扣)`）：

| 状态 | 含义 | 付多少 | 台词 |
|---|---|---|---|
| 0 | 拒绝 | 不成交 | `RfusTalk1/2` 随机 |
| 1 | 接受 | `总价 × (5+tier) / 10` | `AcptTalk1/2` 随机 |
| 2 | 强制接受 | **原价**（`+0x160`，不打折） | `ForceAcptTalkto1/2` + `ForceAcptTalkback` |
| 3 | 严词拒绝 | 不成交 | `HarshTalk1/2`；此后再来买是 `HarshTalkbuy` |

状态 1 和状态 2 的付款分别在 0x460b8f（`钱 -= 折后价`）和 0x461c31
（`钱 -= 原价`）；两支之前都再查一次「钱够不够」，不够就退回状态 0。
付完钱把买的东西逐个塞进背包（`FUN_0040ccf0`），如果买的是装备而队里有人
正穿着同名的，那一件先脱下来（0x460f60 起按角色 0x580 跨度扫六个装备位）。

## 5. 移植

- `shared/src/openpal3/states/shop.rs`：`DealScript`（解析）+ `haggle`
  （掷骰与结算），`pal3_dealscript_census` 拿全部 49 个文件跑一遍。
- `scripting/sce/commands/dealer.rs`：SCE 100 的实现，买卖期间阻塞脚本
  （原版是模式 2 + 等待任务）。界面**不是原版那一套**：原版的五个列表框在
  `FUN_004593c0` / `FUN_00459970` 里按控件基类算相对矩形，还没读通，所以先
  照战斗那样画一个文字浮层。上下选货、左右调数量、确定进还价（`DK == 0`
  的店直接买）、取消离开。
- 还没做：
  - 像素级的界面（同上）；
  - **卖**那一半。`EntrySale` 在数据里，界面也有卖的页，但卖价怎么算还没定
    位：`FUN_00459f30` 是把店铺数据拷进 `+0xcc4` 指向的另一个对象的 `+0x254`
    （买那一页是 `FUN_0045b7a0` 拷到自己的 `+0x178`），钱变多的那一处不在
    0x459000–0x462000 这一段里。**没找到之前不猜**；
  - `Rest`（客栈，SCE 119）那条通路：配置解析有了（`RestScript`），但住店的
    扣钱与回复在 exe 里还没定位——0x44axxx 那一段和当铺 `HockShop` 共用控件。
