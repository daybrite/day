app-title = Day 演示
counter-value = { $count } 次点击
decrement = −
increment = +
name-placeholder = 你的名字
greeting = 你好，{ $name }！
volume-label = 音量
progress-label = 进度
busy-label = 忙碌
flavor-label = 口味
history-title = 历史记录
history-entry = 计数变为 { $value }
nav-controls = 控件
nav-menus = 菜单
nav-text = 文字
nav-battery = 电池
nav-sensors = 传感器
nav-clipboard = 剪贴板
nav-network = 网络
nav-media = 媒体
nav-pickers = 选择器
nav-compose = 组合
nav-files = 文件
nav-tabs = 标签页
nav-stack = 堆栈
nav-list = 列表
nav-webview = 网页视图
nav-lottie = Lottie
nav-about = 关于

shapes-kinds = 种类
shapes-transform = 变换
shapes-angle = 角度

picker-shared-caption = 三种样式绑定到同一个选择信号——改动其中一个，其余的会跟着变化。
picker-selected = 已选
picker-segmented = 分段
picker-menu = 菜单
picker-inline = 内联

compose-caption = 纯组合部件——无原生代码、无 cargo 特性，每个后端都直接可用。
compose-rating-label = 星级评分
compose-rating-count = 已选星数：
compose-rating-placeholder = 1–5
compose-card-title = 可复用的表面
compose-card-body = 内边距 + 背景 + 圆角，以 Modifier 的方式应用。
compose-plain-btn = 普通
compose-styled-btn = 填充
compose-env-value = 使用提供的强调色着色
list-add = 添加 100 行
list-caption = { $count } 行——只构建可见的单元格

webview-url-hint = 输入网址
webview-go = 前往
webview-back = 后退
webview-forward = 前进
webview-stop = 停止
webview-reload = 重新加载

lottie-caption = 原生 Lottie 动画，以 JSON 打包（lottie-ios / lottie-android）
lottie-speed = 速度
stack-root-body = 真正的推入/弹出堆栈。其路径是应用持有的信号。
stack-push = 推入一个详情页
stack-detail-title = 第 { $depth } 层
stack-detail-body = 已推入路径。原生返回按钮会把弹出写回。
stack-item-title = 条目 { $id }
stack-link-42 = 带提示打开 item-42（绝对路由）
stack-param-hint = 携带提示打开：{$hint}
tab-one = 概览
tab-two = 详情
tab-three = 设置
tab-one-body = 概览标签页。每个标签页保留自己的状态。
tab-two-body = 详情标签页，由其路由键选中。
tab-three-body = 设置标签页。深层链接和 dayscript 按键选择标签页。
about-text = 一个用 day 构建的原生跨平台应用。
nav-modals = 模态框
modal-alert = 显示警告框
modal-confirm = 确认
modal-delete = 删除…
modal-sheet = 选择口味
modal-prompt = 输入名字
alert-title = 提示
alert-body = 你的更改已保存。
ok = 好
confirm-title = 退出？
confirm-body = 确定要退出吗？
delete-title = 删除条目？
delete-body = 此操作无法撤销。
delete = 删除
flavor-title = 选择一种口味
cancel = 取消
vanilla = 香草
pistachio = 开心果

# Files playground (docs/files.md)
files-caption = 原生打开/保存文件选择器。打开会把文本文件读入编辑器；保存会把它写回。
files-placeholder = 输入要保存的内容…
files-open = 打开文件…
files-save = 保存文件…
files-opened = 已打开 { $name }

# Battery playground (docs/battery.md)
battery-refresh = 读取设备电池
battery-level = 电量
battery-charging = 充电中
battery-reading = 电池：{ $percent } · { $state }
battery-reading-none = 电池：此平台没有电池 API

# Sensors playground (docs/sensors.md)
sensors-refresh = 读取传感器
sensor-accelerometer = 加速度计
sensor-gyroscope = 陀螺仪
sensor-magnetometer = 磁力计
sensor-reading = x { $x } · y { $y } · z { $z } { $unit }
sensor-waiting = 等待第一个样本…
sensor-unavailable = 此设备上不可用

# Clipboard playground (docs/clipboard.md)
clipboard-caption = day-part-clipboard 部件以原生方式读写系统剪贴板。
clipboard-placeholder = 输入要复制的内容
clipboard-copy = 复制
clipboard-paste = 粘贴
clipboard-idle = 剪贴板未使用
clipboard-copied = 已复制到系统剪贴板
clipboard-copy-failed = 复制失败（此处没有剪贴板 API）
clipboard-pasted = 已从系统剪贴板粘贴
clipboard-empty = 剪贴板为空（或在后台不可读取）

# Network playground (docs/network.md)
network-refresh = 读取网络
network-reading-online = 在线 · { $kind } · 计费：{ $expensive }
network-reading-offline = 离线
network-reading-none = 此平台没有网络连接 API

# Media playground (docs/media.md)
media-play = 播放
media-pause = 暂停
media-load = 加载

# Text playground (typography)
text-caption = 语义样式映射到平台的原生文本样式和无障碍文字缩放。
text-styles-header = 样式
text-weights-header = 字重
text-styling-header = 粗体与斜体
text-colors-header = 颜色
text-custom-header = 自定义字号
text-custom-note = Font.System(pt)——仍按无障碍文字大小缩放（动态字体 / 字体比例）。
text-fonts-header = 打包字体
text-fonts-note = Font.Custom("Family", pt)——来自应用 fonts/ 目录的文件，由 day build 打包并在每个平台按字体族名解析。

# Menus playground
menus-caption = 原生菜单——应用菜单栏与部件的上下文菜单——支持嵌套子菜单、键盘快捷键和标准编辑命令。
menus-last = 最近操作：
menus-lifecycle = 最近生命周期阶段：
menus-context-hint = 上下文菜单
menus-target = 在此右键（移动端长按）打开上下文菜单
menus-shortcut-hint = 键盘快捷键（⌘/Ctrl + 键）显示在菜单栏中，应用聚焦时生效——例如 新建 (N)、保存 (S)、重新加载 (R)、另存为 (⇧S)。

# --- day-part-haptics ---
nav-haptics = 触感反馈
haptics-supported-yes = 此平台有触感引擎
haptics-supported-no = 此平台没有触感引擎（按钮无反馈）
haptics-light = 轻
haptics-medium = 中
haptics-heavy = 重
haptics-success = 成功
haptics-warning = 警告
haptics-error = 错误
haptics-selection = 选择
haptics-last = 最近播放
haptics-none = 尚未播放
haptics-last-played = 已播放：{ $style }

# --- day-part-prefs ---
nav-prefs = 偏好设置
prefs-caption = 使用 day-part-prefs 在多次启动间持久化一个字符串。
prefs-placeholder = 要记住的值
prefs-save = 保存
prefs-load = 读取
prefs-clear = 清除
prefs-idle = 输入一个值，然后保存。
prefs-empty = （未存储任何内容）
prefs-saved = 已保存。
prefs-save-failed = 保存失败。
prefs-loaded = 已从存储读取。
prefs-missing = 尚未存储任何内容。
prefs-cleared = 已清除。
prefs-value-label = 已存储的值：

# --- bundled resources (§18.3) ---
nav-resources = 资源
resources-caption = 按名称从打包资源加载的图片，以及对嵌入数据的随机读取。
resources-numbers = numbers.bin：{ $len } 字节，byte[100] = { $byte }
resources-greeting = greeting.txt：{ $text }

# --- day-part-deviceinfo ---
nav-deviceinfo = 设备信息
deviceinfo-model = 型号：{$value}
deviceinfo-system = 系统：{$name} {$version}
deviceinfo-simulator = 模拟器：{$value}
deviceinfo-yes = 是
deviceinfo-no = 否
deviceinfo-refresh = 刷新

# --- day-piece-activity ---
activity-animating = 动画中
activity-on = 旋转中
activity-off = 已停止

# --- day-piece-searchfield ---
nav-search = 搜索
search-placeholder = 搜索水果…
search-clear = 清除

# --- day-piece-map ---
nav-map = 地图
map-caption = 原生 MKMapView——仅限 Apple 平台。点按预设可实时重新定位地图。
map-sf = 旧金山
map-nyc = 纽约

# — tweaks page (docs/tweaks.md) —
nav-tweaks = 微调
tweaks-intro = 打包的微调按工具包配置内置部件背后的原生控件。在未覆盖的工具包上是空操作——下面的部件只是保持原样。
tweaks-stock = 原样
tweaks-tweaked = 已微调
tweaks-bezel-title = 按钮边框样式
tweaks-bezel-caption = day-tweak-button-bezel——仅 AppKit：在真正的 NSButton 上使用 NSBezelStyle 常量。
tweaks-selectable-title = 可选中标签
tweaks-selectable-caption = day-tweak-label-selectable——AppKit、GTK、Android：在普通标签上启用平台自带的文本选择。
tweaks-selectable-text = 这个标签的文字可以被选中并复制——试试看。
tweaks-ticks-title = 滑块刻度
tweaks-ticks-caption = day-tweak-slider-tickmarks——AppKit、GTK、Android、Qt、WinUI、ArkUI：原生刻度，平台支持时可吸附。微调后的滑块会吸附；原样的滑块平滑滑动。
tweaks-ref-title = NativeRef 存活性
tweaks-ref-caption = NativeRef 在挂载后能访问微调过的滑块；卸载它后引用会被清空而不是悬空。
tweaks-ref-live = 引用：存活
tweaks-ref-cleared = 引用：已清空

# — merged section pages (design overhaul) —
nav-canvas = 画布与形状
nav-system = 设备与传感器
nav-services = 平台服务
controls-caption = 双向绑定：每个控件都是应用持有信号的投影。
controls-basics = 基础
controls-feedback = 反馈
canvas-caption = 形状、变换、手势和组合层部件——全部通过画布绘制。
canvas-gauge = 画布仪表
shapes-interact-hint = 拖动滑块旋转，点按圆形换色，拖动紫色方块移动它。
system-caption = 无界面的设备状态部件：电池、网络连接、运动传感器和设备标识。
services-caption = 无界面的"与操作系统交互"部件：剪贴板、偏好设置、触感反馈和文件选择器。
subscribe-label = 订阅

# — data strings localized for the walkthrough locales (option lists, specimen rows) —
chocolate = 巧克力
size-small = 小
size-medium = 中
size-large = 大
fruit-apple = 苹果
fruit-banana = 香蕉
fruit-cherry = 樱桃
fruit-date = 枣
fruit-elderberry = 接骨木莓
list-row = 第 { $n } 行
text-style-large-title = 大标题
text-style-title = 标题
text-style-title2 = 标题 2
text-style-title3 = 标题 3
text-style-headline = 头条标题
text-style-subheadline = 副标题
text-style-body = 正文
text-style-callout = 标注
text-style-footnote = 脚注
text-style-caption = 说明文字
text-style-caption2 = 说明文字 2
text-weight-ultralight = 极细
text-weight-light = 细体
text-weight-regular = 常规
text-weight-medium = 中等
text-weight-semibold = 半粗
text-weight-bold = 粗体
text-weight-heavy = 特粗
text-weight-black = 黑体
text-bold = 粗体文本
text-italic = 斜体文本
text-bolditalic = 粗斜体
text-emphasis-label = 强调
color-red = 红
color-green = 绿
color-blue = 蓝
color-orange = 橙
