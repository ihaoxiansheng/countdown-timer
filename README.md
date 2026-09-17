# 简易倒计时

一个常驻屏幕角落的极简倒计时小挂件。没有标题栏、没有按钮、没有设置面板——整个窗口就是一块大号数字，左键点一下开始或暂停，右键弹出菜单。

跨平台支持 **macOS** 与 **Windows**，基于 Rust + Tauri 2 构建，安装包体积不到 2 MB。

![License](https://img.shields.io/badge/license-Apache--2.0-blue)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey)
![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB)

---

## 目录

- [特性](#特性)
- [下载安装](#下载安装)
- [使用说明](#使用说明)
- [界面配色](#界面配色)
- [从源码构建](#从源码构建)
- [项目结构](#项目结构)
- [设计要点](#设计要点)
- [自动构建](#自动构建-github-actions)
- [常见问题](#常见问题)
- [开源协议](#开源协议)

---

## 特性

**核心计时**

- **倒计时** —— 15 档常用预设（1/2/3/5/10/15/20/25/30/35/40/45/50/60/90 分），或手动输入任意时长
- **超时正计时** —— 归零后不停表，自动转红继续向上累计，直观显示「超了多久」
- **秒表模式** —— 从 `00:00` 起步向上走，没有目标时长
- **零漂移** —— 计时基于绝对时刻反算，而非逐帧累加；系统休眠、线程挂起、定时器抖动都不会让时间走偏

**窗口交互**

- **无边框 + 圆角 + 阴影** —— 贴在屏幕任意角落都不突兀
- **默认置顶** —— 全屏写代码/看视频时也不会被盖住，可随时关闭
- **任意位置拖动** —— 按住窗口任何地方即可挪动，不需要瞄准标题栏
- **自由缩放** —— 数字字号随窗口尺寸实时换算，拖多大都清晰；一键「重置窗口」缩回小挂件尺寸

**提醒**

- **结束提醒音** —— 默认关闭，可在右键菜单开启；开启后归零响 3 声（前 2 秒内）
- **原生系统音** —— macOS 用 `Glass.aiff`，Windows 用系统蜂鸣，不打包任何音频文件

---

## 下载安装

前往 [Releases](https://github.com/ihaoxiansheng/countdown-timer/releases) 或 [Actions 构建产物](https://github.com/ihaoxiansheng/countdown-timer/actions) 下载。

| 平台 | 文件 | 说明 |
|---|---|---|
| **macOS（Apple Silicon）** | `macos-aarch64-dmg` | M1/M2/M3/M4 芯片 |
| **macOS（Intel）** | `macos-x86_64-dmg` | Intel 芯片 |
| **Windows** | `windows-installer` | 安装程序（NSIS，简体中文） |
| **Windows** | `windows-portable` | 免安装单文件 exe |

**系统要求**：macOS 13.0 (Ventura) 或更高；Windows 10 / 11。

### macOS 首次打开提示「无法验证开发者」

应用没有 Apple 开发者签名，首次打开会被 Gatekeeper 拦下。任选一种方式放行：

```bash
# 方式一:命令行移除隔离标记
xattr -cr /Applications/简易倒计时.app
```

方式二：「系统设置 → 隐私与安全性」，在底部找到被拦截的提示，点「仍要打开」。

---

## 使用说明

### 鼠标

| 操作 | 效果 |
|---|---|
| **左键单击** | 开始 / 暂停 / 继续 |
| **左键拖动** | 移动窗口 |
| **右键单击** | 弹出菜单 |
| **拖动窗口边缘** | 缩放窗口，数字字号自动跟随 |

> 响铃期间的第一次左键单击只用来「让它安静」，计时不会被打断；再点一次才是暂停。这样「到点了想静音」和「想停表」是两个独立动作，不会误触。

### 键盘

| 按键 | 效果 |
|---|---|
| <kbd>空格</kbd> | 开始 / 暂停 / 继续 |
| <kbd>R</kbd> 或 <kbd>Esc</kbd> | 重置 |

### 右键菜单

```
开始 / 暂停 / 继续          ← 文案随当前状态变化
重置
─────────────────
设置时间  ▸
    自定义…
    ─────────────
    1分  2分  3分  5分  10分  15分
    20分 25分 30分 35分 40分 45分
    50分 60分 90分            ← 当前选中项打勾
─────────────────
开始正向计时                ← 秒表模式
结束提醒音                  ← 开关,默认关闭
─────────────────
重置窗口                    ← 缩回 110×44 最小尺寸
─────────────────
窗口置顶                    ← 开关,默认开启
─────────────────
退出
```

### 自定义时间格式

选择「设置时间 → 自定义…」后，输入框支持以下写法：

| 输入 | 解析结果 | 说明 |
|---|---|---|
| `25` | 25 分钟 | 纯数字按分钟计算 |
| `05:30` | 5 分 30 秒 | `分:秒` |
| `1:20:00` | 1 小时 20 分 | `时:分:秒` |
| `5:` | 5 分钟 | 省略的段按 0 计 |
| `:30` | 30 秒 | 同上 |
| `05：30` | 5 分 30 秒 | 全角冒号自动转换 |

上限 **90:00**，超出部分自动按 90:00 处理，不报错也不打断操作。确认后立即开始倒计时。

---

## 界面配色

数字颜色即状态指示，不需要额外的文字标签：

| 状态 | 数字颜色 | 进度条 | 附加表现 |
|---|---|---|---|
| **空闲** | ⚫️ 黑色 | 青色，停在当前比例 | —— |
| **倒计时中** | 🟢 绿色 `#34c759` | 青色 `#30b0c7`，逐渐消耗 | —— |
| **已暂停** | 🟠 橙色 `#ff9500` | 橙色 | 数字压暗至 45%，正中叠加播放三角 |
| **超时** | 🔴 红色 `#ff3b30` | 红色，整条铺满 | 数字向上累计 |
| **秒表** | 🟢 绿色 | 留空（无目标时长） | 数字向上累计 |

---

## 从源码构建

### 环境准备

| 依赖 | 版本 | 说明 |
|---|---|---|
| [Rust](https://rustup.rs/) | 1.77+ | 含 `cargo` |
| [Node.js](https://nodejs.org/) | 20+ | 仅用于跑 Tauri CLI |
| **macOS** | Xcode Command Line Tools | `xcode-select --install` |
| **Windows** | Visual Studio Build Tools | 勾选「使用 C++ 的桌面开发」 |
| **Windows** | WebView2 Runtime | Win11 已内置，Win10 需[单独安装](https://developer.microsoft.com/microsoft-edge/webview2/) |

### 命令

```bash
# 克隆
git clone https://github.com/ihaoxiansheng/countdown-timer.git
cd countdown-timer

# 安装依赖(只有 Tauri CLI 一个)
npm install

# 开发模式:热重载,带控制台日志
npm run tauri dev

# 发布构建:产物在 src-tauri/target/release/bundle/
npm run tauri build

# 跑单元测试(14 个,覆盖格式化、解析、状态流转)
cd src-tauri && cargo test
```

> 前端没有任何构建步骤——`src/` 下的 HTML/CSS/JS 被 Tauri 直接打包，改完刷新即可，不需要 Vite、webpack 或 npm 脚本。

### 构建产物位置

```
src-tauri/target/release/bundle/
├── dmg/简易倒计时_1.0.0_aarch64.dmg   # macOS 安装镜像
├── macos/简易倒计时.app                # macOS 应用包
└── nsis/简易倒计时_1.0.0_x64-setup.exe # Windows 安装程序
```

指定 target 交叉编译时（如 `--target x86_64-apple-darwin`），产物路径会变成 `target/<target>/release/bundle/`。

---

## 项目结构

```
countdown-timer/
├── src/                      # 前端(无构建步骤,直接打包)
│   ├── index.html            #   28 行,整个 DOM 结构
│   ├── styles.css            #  137 行,配色与版面
│   └── main.js               #  219 行,渲染 + 字号换算 + 事件
│
├── src-tauri/                # Rust 后端
│   ├── src/
│   │   ├── main.rs           #  447 行,窗口/菜单/推帧/提示音
│   │   └── timer.rs          #  523 行,纯计时逻辑 + 14 个单元测试
│   ├── icons/                # 各平台图标(icns / ico / png)
│   ├── capabilities/         # Tauri 2 权限声明
│   ├── tauri.conf.json       # 窗口尺寸、打包目标、CSP
│   └── Cargo.toml
│
├── .github/workflows/
│   └── build.yml             # 三平台自动构建
│
├── package.json              # 只有 @tauri-apps/cli 一个依赖
├── LICENSE                   # Apache 2.0
└── README.md
```

### 技术栈

| 层 | 选型 | 理由 |
|---|---|---|
| 界面 | 原生 HTML / CSS / JS | 界面只有一个数字和一根进度条，上框架是负收益 |
| 逻辑 | Rust | 状态机集中一处，两平台行为不会各自漂移 |
| 外壳 | Tauri 2 | 复用系统 WebView，产物不到 2 MB（Electron 同等功能约 80 MB） |
| 对话框 | `osascript` / PowerShell | Tauri 没有内置文本输入框，调系统原生的，零依赖 |
| 构建 | GitHub Actions | 在 macOS 上无法交叉编译 Windows exe |

---

## 设计要点

### 状态全放 Rust，前端只负责画

后端每 100 ms 推一帧 `{ text, progress, phase }` 给前端，前端不做任何时间计算——连 `00:05` 这样的字符串都是 Rust 拼好的。这样 macOS 和 Windows 跑的是**同一份逻辑**，不会出现「Windows 上少跳一秒」这类平台差异。

### 为什么用后台线程推帧，而不是前端 `setInterval`

窗口最小化或失焦时，浏览器会把 JS 定时器降频到每秒甚至更慢。计时器不能因为「你没看着它」就走慢，所以推帧交给 Rust 的独立线程。

### 零漂移计时

不靠「每帧 `remaining -= 0.1`」累加，而是记录**绝对时刻**再反算：

```rust
// 倒计时:记结束时刻,每帧算 end_at - now
self.end_at = Some(Instant::now() + Duration::from_secs_f64(self.remaining));

// 正计时:记起点时刻,每帧算 now - up_from
self.up_from = Some(Instant::now() - Duration::from_secs_f64(self.elapsed_up));
```

暂停时把当前值落成快照，恢复时把基准时刻反向平移回去。误差不会随时间累积。

### 归零瞬间不丢秒

倒计时跨零的那一帧，实际已经越过零点一小段时间（`overshoot`）。直接从 0 开始计超时会丢掉这段：

```rust
let overshoot = now.duration_since(end_at).as_secs_f64();
// 把正计时起点往前挪 overshoot,显示值从 overshoot 起算而不是 0
self.up_from = Some(now - Duration::from_secs_f64(overshoot));
```

### 数字字号不抖动

字号需要按窗口尺寸实时换算，但如果每帧测量**当前文案**的宽度，`00:09` → `00:10` 的宽度差会让字号突然跳变，看起来就是数字在呼吸。

解决办法是按**字符数**选用固定参考串测量，而不是测真实文案：

```javascript
// "88:88" 对应所有 5 字符文案,"8:88:88" 对应 7 字符文案
const base = charCount <= 5 ? baseMetrics5 : baseMetrics7;
const scale = Math.min(bandH / base.capHeight, usableW / base.width);
```

配合 CSS 的 `font-variant-numeric: tabular-nums`（等宽数字），同字符数的串宽度完全一致，字号全程稳定。

### 圆角是自己画的

`decorations: false` 去掉标题栏的同时，系统也不再给窗口裁圆角。恢复圆角需要三件事配齐，缺一不可：

1. `Cargo.toml` 开 `macos-private-api` 特性
2. `tauri.conf.json` 里 `macOSPrivateApi: true` + 窗口 `transparent: true`
3. CSS 里 `body` 透明、内层容器 `border-radius` + `overflow: hidden`

第三步的 `overflow: hidden` 是为了让底部进度条的两个下角也被裁进圆弧，否则进度条会以直角戳出来。

### 拖动与点击的边界

整个窗口都是 `data-tauri-drag-region` 拖动区。但拖动命中判定看的是 **mousedown 落在哪个元素上**，而数字层盖住了窗口中部——所以数字和播放三角都设了 `pointer-events: none`，让事件穿透到底层容器。

点击则通过位移阈值与拖动区分：按下与抬起之间位移小于 3 px 才算单击，避免手抖导致点击失效。

---

## 自动构建 (GitHub Actions)

推送代码后自动构建三个平台，产物可直接下载。

**触发方式**

```bash
# 方式一:推送到 main / master 分支
git push

# 方式二:打标签(推荐用于正式发布)
git tag v1.0.0 && git push origin v1.0.0

# 方式三:GitHub 页面 Actions → Build Apps → Run workflow
```

**构建矩阵**

| 运行环境 | 目标架构 | 产物 |
|---|---|---|
| `macos-latest` | `aarch64-apple-darwin` | dmg + app |
| `macos-latest` | `x86_64-apple-darwin` | dmg + app |
| `windows-latest` | `x86_64-pc-windows-msvc` | 安装程序 + 免安装 exe |

耗时约 5–10 分钟。构建完成后在 workflow 页面底部的 **Artifacts** 区域下载，保留 90 天。

### 发布新版本

版本号必须符合语义化版本规范（`主版本.次版本.修订号`，**不能写 `1.0`**），且需要三处同步修改：

```
package.json              → "version": "1.1.0"
src-tauri/tauri.conf.json → "version": "1.1.0"
src-tauri/Cargo.toml      → version = "1.1.0"
```

```bash
git add . && git commit -m "chore: bump version to 1.1.0"
git tag v1.1.0 && git push origin master --tags
```

---

## 常见问题

**Q：窗口太小/太大了怎么办？**
拖动窗口边缘即可缩放，数字字号自动跟随。想快速缩回小挂件尺寸，右键选「重置窗口」。

**Q：怎么让它不挡住其他窗口？**
右键取消勾选「窗口置顶」。

**Q：为什么到点了没有声音？**
提醒音默认关闭。右键勾选「结束提醒音」即可，之后归零会响 3 声（前 2 秒内）。注意这个开关不会持久化，重启应用后恢复默认关闭。

**Q：倒计时归零后为什么还在走？**
这是刻意设计的超时计时——数字转红向上累计，告诉你「已经超了多久」。点一下或按空格即可停下。

**Q：能设置超过 90 分钟吗？**
不能，上限就是 90:00。超出的输入会自动按 90:00 处理。

**Q：关掉应用后设置会保存吗？**
不会。当前版本所有状态（时长、置顶、提醒音）都在内存中，重启后回到默认值。

**Q：构建失败，提示 `package > version must be a semver string`？**
版本号写成了 `1.0` 这种两段式。Tauri 要求严格的三段 semver，改成 `1.0.0`。

**Q：Windows 构建成功但下载不到 exe？**
检查 `build.yml` 里的产物路径。使用 `--target` 参数编译时，输出目录是 `src-tauri/target/<target>/release/`，而不是 `src-tauri/target/release/`。

---

## 开源协议

本项目基于 [Apache License 2.0](LICENSE) 开源。

```
Copyright 2026 xu.liang

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0
```
