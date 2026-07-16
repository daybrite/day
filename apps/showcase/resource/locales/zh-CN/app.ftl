app_title = Day 演示
counter_value = { $count } 次点击
decrement = −
increment = +
name_placeholder = 你的名字
greeting = 你好，{ $name }！
volume_label = 音量
progress_label = 进度
busy_label = 忙碌
flavor_label = 口味
history_title = 历史记录
history_entry = 计数变为 { $value }
nav_controls = 控件
nav_menus = 菜单
nav_text = 文字
nav_battery = 电池
nav_sensors = 传感器
nav_clipboard = 剪贴板
nav_network = 网络
nav_media = 媒体
nav_pickers = 选择器
nav_compose = 组合
nav_files = 文件
nav_tabs = 标签页
nav_stack = 堆栈
nav_list = 列表
nav_webview = 网页视图
nav_lottie = Lottie
nav_about = 关于

shapes_kinds = 种类
gradients_title = 渐变
gradient_angle = 角度
shapes_transform = 变换
shapes_angle = 角度

picker_shared_caption = 三种样式绑定到同一个选择信号——改动其中一个，其余的会跟着变化。
picker_selected = 已选
picker_segmented = 分段
picker_menu = 菜单
picker_inline = 内联

compose_caption = 纯组合部件——无原生代码、无 cargo 特性，每个后端都直接可用。
compose_rating_label = 星级评分
compose_rating_count = 已选星数：
compose_rating_placeholder = 1–5
compose_card_title = 可复用的表面
compose_card_body = 内边距 + 背景 + 圆角，以 Modifier 的方式应用。
compose_plain_btn = 普通
compose_styled_btn = 填充
compose_env_value = 使用提供的强调色着色
list_add = 添加 100 行
list_caption = { $count } 行——只构建可见的单元格

webview_url_hint = 输入网址
webview_go = 前往
webview_back = 后退
webview_forward = 前进
webview_stop = 停止
webview_reload = 重新加载

lottie_caption = 原生 Lottie 动画，以 JSON 打包（lottie-ios / lottie-android）
lottie_speed = 速度
stack_root_body = 真正的推入/弹出堆栈。其路径是应用持有的信号。
stack_push = 推入一个详情页
stack_detail_title = 第 { $depth } 层
stack_detail_body = 已推入路径。原生返回按钮会把弹出写回。
stack_item_title = 条目 { $id }
stack_link_42 = 带提示打开 item-42（绝对路由）
stack_param_hint = 携带提示打开：{$hint}
tab_one = 概览
tab_two = 详情
tab_three = 设置
tab_one_body = 概览标签页。每个标签页保留自己的状态。
tab_two_body = 详情标签页，由其路由键选中。
tab_three_body = 设置标签页。深层链接和 dayscript 按键选择标签页。
about_text = 一个用 day 构建的原生跨平台应用。
nav_modals = 模态框
modal_alert = 显示警告框
modal_confirm = 确认
modal_delete = 删除…
modal_sheet = 选择口味
modal_prompt = 输入名字
alert_title = 提示
alert_body = 你的更改已保存。
ok = 好
confirm_title = 退出？
confirm_body = 确定要退出吗？
delete_title = 删除条目？
delete_body = 此操作无法撤销。
delete = 删除
flavor_title = 选择一种口味
cancel = 取消
vanilla = 香草
pistachio = 开心果

# Files playground (docs/files.md)
files_caption = 原生打开/保存文件选择器。打开会把文本文件读入编辑器；保存会把它写回。
files_placeholder = 输入要保存的内容…
files_open = 打开文件…
files_save = 保存文件…
files_opened = 已打开 { $name }

# Battery playground (docs/battery.md)
battery_refresh = 读取设备电池
battery_level = 电量
battery_charging = 充电中
battery_reading = 电池：{ $percent } · { $state }
battery_reading_none = 电池：此平台没有电池 API

# Sensors playground (docs/sensors.md)
sensors_refresh = 读取传感器
sensor_accelerometer = 加速度计
sensor_gyroscope = 陀螺仪
sensor_magnetometer = 磁力计
sensor_reading = x { $x } · y { $y } · z { $z } { $unit }
sensor_waiting = 等待第一个样本…
sensor_unavailable = 此设备上不可用

# Clipboard playground (docs/clipboard.md)
clipboard_caption = day-part-clipboard 部件以原生方式读写系统剪贴板。
clipboard_placeholder = 输入要复制的内容
clipboard_copy = 复制
clipboard_paste = 粘贴
clipboard_idle = 剪贴板未使用
clipboard_copied = 已复制到系统剪贴板
clipboard_copy_failed = 复制失败（此处没有剪贴板 API）
clipboard_pasted = 已从系统剪贴板粘贴
clipboard_empty = 剪贴板为空（或在后台不可读取）

# Network playground (docs/network.md)
network_refresh = 读取网络
network_reading_online = 在线 · { $kind } · 计费：{ $expensive }
network_reading_offline = 离线
network_reading_none = 此平台没有网络连接 API

# Media playground (docs/media.md)
media_play = 播放
media_pause = 暂停
media_load = 加载

# Text playground (typography)
text_caption = 语义样式映射到平台的原生文本样式和无障碍文字缩放。
text_styles_header = 样式
text_weights_header = 字重
text_styling_header = 粗体与斜体
text_colors_header = 颜色
text_custom_header = 自定义字号
text_custom_note = Font.System(pt)——仍按无障碍文字大小缩放（动态字体 / 字体比例）。
text_fonts_header = 打包字体
text_fonts_note = Font.Custom("Family", pt)——来自应用 resource/fonts/ 目录的文件，由 day build 打包并在每个平台按字体族名解析。

# Menus playground
menus_caption = 原生菜单——应用菜单栏与部件的上下文菜单——支持嵌套子菜单、键盘快捷键和标准编辑命令。
menus_last = 最近操作：
menus_lifecycle = 最近生命周期阶段：
menus_context_hint = 上下文菜单
menus_target = 在此右键（移动端长按）打开上下文菜单
menus_shortcut_hint = 键盘快捷键（⌘/Ctrl + 键）显示在菜单栏中，应用聚焦时生效——例如 新建 (N)、保存 (S)、重新加载 (R)、另存为 (⇧S)。

# --- day-part-haptics ---
nav_haptics = 触感反馈
haptics_supported_yes = 此平台有触感引擎
haptics_supported_no = 此平台没有触感引擎（按钮无反馈）
haptics_light = 轻
haptics_medium = 中
haptics_heavy = 重
haptics_success = 成功
haptics_warning = 警告
haptics_error = 错误
haptics_selection = 选择
haptics_last = 最近播放
haptics_none = 尚未播放
haptics_last_played = 已播放：{ $style }

# --- day-part-prefs ---
nav_prefs = 偏好设置
prefs_caption = 使用 day-part-prefs 在多次启动间持久化一个字符串。
prefs_placeholder = 要记住的值
prefs_save = 保存
prefs_load = 读取
prefs_clear = 清除
prefs_idle = 输入一个值，然后保存。
prefs_empty = （未存储任何内容）
prefs_saved = 已保存。
prefs_save_failed = 保存失败。
prefs_loaded = 已从存储读取。
prefs_missing = 尚未存储任何内容。
prefs_cleared = 已清除。
prefs_value_label = 已存储的值：

# --- bundled resources (§18.3) ---
nav_resources = 资源
resources_caption = 按名称从打包资源加载的图片，以及对嵌入数据的随机读取。
resources_numbers = numbers.bin：{ $len } 字节，byte[100] = { $byte }
resources_greeting = greeting.txt：{ $text }

# --- day-part-deviceinfo ---
nav_deviceinfo = 设备信息
deviceinfo_model = 型号：{$value}
deviceinfo_system = 系统：{$name} {$version}
deviceinfo_simulator = 模拟器：{$value}
deviceinfo_yes = 是
deviceinfo_no = 否
deviceinfo_refresh = 刷新

# --- day-piece-activity ---
activity_animating = 动画中
activity_on = 旋转中
activity_off = 已停止

# --- day-piece-searchfield ---
nav_search = 搜索
search_placeholder = 搜索水果…
search_clear = 清除

# --- day-piece-map ---
nav_map = 地图
map_caption = 原生 MKMapView——仅限 Apple 平台。点按预设可实时重新定位地图。
map_sf = 旧金山
map_nyc = 纽约

# — tweaks page (docs/tweaks.md) —
nav_tweaks = 微调
tweaks_intro = 打包的微调按工具包配置内置部件背后的原生控件。在未覆盖的工具包上是空操作——下面的部件只是保持原样。
tweaks_stock = 原样
tweaks_tweaked = 已微调
tweaks_bezel_title = 按钮边框样式
tweaks_bezel_caption = day-tweak-button-bezel——仅 AppKit：在真正的 NSButton 上使用 NSBezelStyle 常量。
tweaks_selectable_title = 可选中标签
tweaks_selectable_caption = day-tweak-label-selectable——AppKit、GTK、Android：在普通标签上启用平台自带的文本选择。
tweaks_selectable_text = 这个标签的文字可以被选中并复制——试试看。
tweaks_ticks_title = 滑块刻度
tweaks_ticks_caption = day-tweak-slider-tickmarks——AppKit、GTK、Android、Qt、WinUI、ArkUI：原生刻度，平台支持时可吸附。微调后的滑块会吸附；原样的滑块平滑滑动。
tweaks_ref_title = NativeRef 存活性
tweaks_ref_caption = NativeRef 在挂载后能访问微调过的滑块；卸载它后引用会被清空而不是悬空。
tweaks_ref_live = 引用：存活
tweaks_ref_cleared = 引用：已清空

# — merged section pages (design overhaul) —
nav_canvas = 画布与形状
nav_system = 设备与传感器
nav_services = 平台服务
controls_caption = 双向绑定：每个控件都是应用持有信号的投影。
controls_basics = 基础
controls_feedback = 反馈
canvas_caption = 形状、变换、手势和组合层部件——全部通过画布绘制。
canvas_gauge = 画布仪表
shapes_interact_hint = 拖动滑块旋转，点按圆形换色，拖动紫色方块移动它。
system_caption = 无界面的设备状态部件：电池、网络连接、运动传感器和设备标识。
services_caption = 无界面的"与操作系统交互"部件：剪贴板、偏好设置、触感反馈和文件选择器。
subscribe_label = 订阅

# — data strings localized for the walkthrough locales (option lists, specimen rows) —
chocolate = 巧克力
size_small = 小
size_medium = 中
size_large = 大
fruit_apple = 苹果
fruit_banana = 香蕉
fruit_cherry = 樱桃
fruit_date = 枣
fruit_elderberry = 接骨木莓
list_row = 第 { $n } 行
text_style_large_title = 大标题
text_style_title = 标题
text_style_title2 = 标题 2
text_style_title3 = 标题 3
text_style_headline = 头条标题
text_style_subheadline = 副标题
text_style_body = 正文
text_style_callout = 标注
text_style_footnote = 脚注
text_style_caption = 说明文字
text_style_caption2 = 说明文字 2
text_weight_ultralight = 极细
text_weight_light = 细体
text_weight_regular = 常规
text_weight_medium = 中等
text_weight_semibold = 半粗
text_weight_bold = 粗体
text_weight_heavy = 特粗
text_weight_black = 黑体
text_bold = 粗体文本
text_italic = 斜体文本
text_bolditalic = 粗斜体
text_emphasis_label = 强调
color_red = 红
color_green = 绿
color_blue = 蓝
color_orange = 橙
