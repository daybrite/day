app_title = عرض Day
counter_value = { $count ->
    [zero] لا نقرات
    [one] نقرة واحدة
    [two] نقرتان
    [few] { $count } نقرات
   *[other] { $count } نقرة
}
decrement = −
increment = +
name_placeholder = اسمك
greeting = مرحبًا، { $name }!
volume_label = مستوى الصوت
progress_label = التقدّم
busy_label = مشغول
flavor_label = النكهة
flavor_placeholder = اكتب نكهة أو اخترها
flavor_add = إضافة
flavor_ios_note = لا يوفر iOS عنصر مربع تحرير وسرد، لذا يعرض Day عنصرًا نائبًا هنا.
history_title = السجلّ
history_entry = أصبح العدّاد { $value }
nav_controls = عناصر التحكّم
nav_menus = القوائم والحوارات
nav_text = النص
nav_battery = البطارية
nav_sensors = المستشعرات
nav_clipboard = الحافظة
nav_network = الشبكة
nav_media = الوسائط
nav_pickers = المنتقيات
nav_compose = التركيب
nav_files = الملفات
nav_tabs = علامات التبويب
nav_stack = المكدّس
nav_list = القائمة
nav_refresh = تحديث
refresh_caption = اسحب القائمة للأسفل — أو استخدم الزر — لإعادة التحميل
refresh_status_idle = خامل
refresh_status_refreshing = جارٍ التحديث…
refresh_now = حدّث الآن
refresh_tier_native = السحب للتحديث: أصلي
refresh_tier_emulated = السحب للتحديث: محاكى
refresh_row = عنصر { $n }
nav_webview = عرض الويب
nav_lottie = Lottie
nav_about = حول

shapes_kinds = الأنواع
gradients_title = التدرجات اللونية
gradient_angle = الزاوية
shapes_transform = التحويل
shapes_angle = الزاوية

picker_shared_caption = الأنماط الثلاثة مرتبطة بإشارة اختيار واحدة — غيّر أحدها وسيتبعه الآخران.
picker_selected = المحدد
picker_segmented = مقسّم
picker_menu = قائمة
picker_inline = مضمّن

# — day-piece-datetime —
nav_dates = التاريخ والوقت
dates_caption = منتقيات تاريخ ووقت أصلية مرتبطة باتجاهين بإشارات مدنية — منتقيات القسم الواحد تتشارك الإشارة نفسها.
dates_date_section = التاريخ
dates_time_section = الوقت
dates_composed_section = مركّب
date_compact = مضغوط
date_inline = التقويم
time_compact = مضغوط
time_seconds = بالثواني
dates_composed = التاريخ والوقت
date_bounded = ضمن 2026
date_picked = التاريخ المختار
time_picked = الوقت المختار

compose_caption = قطع تركيبية بحتة — بلا شيفرة أصلية ولا ميزات cargo، وتعمل على كل واجهة خلفية مباشرة.
compose_rating_label = تقييم بالنجوم
compose_rating_count = النجوم المحدّدة:
compose_rating_placeholder = ١–٥
compose_card_title = سطح قابل لإعادة الاستخدام
compose_card_body = حشوة + خلفية + زوايا مستديرة، تُطبَّق كمُعدِّل.
compose_plain_btn = عادي
compose_styled_btn = ممتلئ
compose_env_value = ملوَّن باللون المميّز المُمرَّر
list_add = أضف ١٠٠
list_caption = { $count } صف — تُبنى الخلايا المرئية فقط

webview_url_hint = أدخل عنوان URL
webview_go = انتقال
webview_back = رجوع
webview_forward = تقدّم
webview_stop = إيقاف
webview_reload = إعادة تحميل

lottie_caption = رسم Lottie متحرّك أصلي، مضمَّن كملف JSON‏ (lottie-ios / lottie-android)
lottie_speed = السرعة
stack_root_body = مكدّس دفع/سحب حقيقي. مساره إشارة يملكها التطبيق.
stack_push = ادفع صفحة تفاصيل
stack_detail_title = المستوى { $depth }
stack_detail_body = دُفع إلى المسار. زر الرجوع الأصلي يكتب السحب مرة أخرى.
stack_item_title = العنصر { $id }
stack_link_42 = افتح item-42 مع تلميح (مسار مطلق)
stack_param_hint = فُتح مع تلميح: {$hint}
tab_one = نظرة عامة
tab_two = التفاصيل
tab_three = الإعدادات
tab_one_body = علامة تبويب النظرة العامة. تحتفظ كل علامة تبويب بحالتها الخاصة.
tab_two_body = علامة تبويب التفاصيل، تُحدَّد بمفتاح مسارها.
tab_three_body = علامة تبويب الإعدادات. الروابط العميقة وdayscript يحدّدان علامات التبويب بالمفتاح.
about_text = تطبيق أصلي متعدد المنصات مبني بـ day.
modal_alert = إظهار تنبيه
modal_confirm = تأكيد
modal_delete = حذف…
modal_sheet = اختر نكهة
modal_prompt = أدخل الاسم
alert_title = إشعار
alert_body = حُفظت تغييراتك.
ok = حسنًا
confirm_title = إنهاء؟
confirm_body = هل أنت متأكد أنك تريد الإنهاء؟
delete_title = حذف العنصر؟
delete_body = لا يمكن التراجع عن هذا.
delete = حذف
flavor_title = اختر نكهة
cancel = إلغاء
vanilla = فانيليا
pistachio = فستق

# Files playground (docs/files.md)
files_caption = منتقيات فتح/حفظ الملفات الأصلية. «فتح» يقرأ ملفًا نصيًا إلى المحرّر؛ و«حفظ» يكتبه مرة أخرى.
files_placeholder = اكتب شيئًا لحفظه…
files_open = فتح ملف…
files_save = حفظ الملف…
files_opened = فُتح { $name }

# Battery playground (docs/battery.md)
battery_refresh = قراءة بطارية الجهاز
battery_level = المستوى
battery_charging = يُشحن
battery_reading = البطارية: { $percent } · { $state }
battery_reading_none = البطارية: لا توجد واجهة برمجة للبطارية على هذه المنصة

# Sensors playground (docs/sensors.md)
sensors_refresh = قراءة المستشعرات
sensor_accelerometer = مقياس التسارع
sensor_gyroscope = الجيروسكوب
sensor_magnetometer = مقياس المغناطيسية
sensor_reading = x { $x } · y { $y } · z { $z } { $unit }
sensor_waiting = في انتظار أول عيّنة…
sensor_unavailable = غير متوفر على هذا الجهاز

# Clipboard playground (docs/clipboard.md)
clipboard_caption = قطعة day-part-clipboard تقرأ الحافظة النظامية وتكتب فيها بشكل أصلي.
clipboard_placeholder = اكتب شيئًا لنسخه
clipboard_copy = نسخ
clipboard_paste = لصق
clipboard_idle = الحافظة لم تُمسّ
clipboard_copied = نُسخ إلى الحافظة النظامية
clipboard_copy_failed = فشل النسخ (لا توجد واجهة حافظة هنا)
clipboard_pasted = لُصق من الحافظة النظامية
clipboard_empty = الحافظة فارغة (أو غير قابلة للقراءة في الخلفية)

# Network playground (docs/network.md)
network_refresh = قراءة الشبكة
network_reading_online = متصل · { $kind } · مقنَّن: { $expensive }
network_reading_offline = غير متصل
network_reading_none = لا توجد واجهة اتصال على هذه المنصة

# Media playground (docs/media.md)
media_play = تشغيل
media_pause = إيقاف مؤقت
media_load = تحميل

# — Localization page (docs/localization.md) —
nav_localization = التوطين
fmt_caption = مجموعة ترجمات واحدة — عرض مطابق لـ ICU لكل لغة: الأرقام والتواريخ وقواعد الجمع وترتيب الفرز كلها تتبع اللغة.
loc_locale_section = اللغة الحية
loc_live_note = اللغة إشارة — تبديلها يعيد عرض كل النصوص فورًا. اتجاه التخطيط يثبت عند الإطلاق (أطلق بالعربية لواجهة معكوسة).
loc_current_label = الحالية
loc_reset = إعادة الضبط
loc_numbers_section = الأرقام
loc_dates_section = التواريخ والأوقات
loc_plurals_section = الجموع
loc_sorting_section = الفرز
fmt_number_label = مجمّع
fmt_fraction_label = منزلتان عشريتان
fmt_percent_label = النسبة المئوية
fmt_date_label = تاريخ طويل
fmt_time_label = الوقت
fmt_datetime_label = التاريخ والوقت
fmt_sorted_label = مرتّب
fmt_number = { NUMBER($n) }
fmt_fraction = { NUMBER($n, minimumFractionDigits: 2) }
fmt_percent = { NUMBER($p, style: "percent") }
fmt_date = { DATETIME($d, dateStyle: "long") }
fmt_time = { DATETIME($t, timeStyle: "short") }
fmt_datetime = { DATETIME($dt, dateStyle: "medium", timeStyle: "short") }
plural_items = { $count ->
    [zero] لا عناصر
    [one] عنصر واحد
    [two] عنصران
    [few] { $count } عناصر
    [many] { $count } عنصرًا
   *[other] { $count } عنصر
}

# Text playground (typography)
text_caption = الأنماط الدلالية تُطابق أنماط النص الأصلية للمنصة وتكبير نص إمكانية الوصول.
text_styles_header = الأنماط
text_weights_header = الأوزان
text_styling_header = عريض ومائل
text_colors_header = اللون
text_custom_header = أحجام مخصّصة
text_custom_note = ‏Font.System(pt) — لا يزال يُكبَّر وفق حجم نص إمكانية الوصول (الخط الديناميكي / مقياس الخط).
text_fonts_header = الخطوط المضمّنة
text_fonts_note = ‏Font.Custom("Family", pt) — ملفات من مجلد resource/fonts/ في التطبيق، يضمّها day build وتُحلّ باسم العائلة على كل منصة.

# Menus playground
menus_caption = قوائم أصلية — شريط قوائم التطبيق وقوائم السياق لكل قطعة — مع قوائم فرعية متداخلة واختصارات لوحة المفاتيح وأوامر التحرير القياسية.
menus_last = آخر إجراء
menus_lifecycle = دورة الحياة
menus_target = انقر بزر الفأرة الأيمن هنا (ضغطة مطوّلة على الجوال) لقائمة سياق
menus_shortcut_hint = اختصارات لوحة المفاتيح (⌘/Ctrl + مفتاح) تظهر في شريط القوائم وتعمل أثناء تركيز التطبيق — مثل جديد (N)، حفظ (S)، إعادة تحميل (R)، حفظ باسم (⇧S).

# --- day-part-haptics ---
nav_haptics = الاهتزاز اللمسي
haptics_supported_yes = محرك الاهتزاز اللمسي متوفر على هذه المنصة
haptics_supported_no = لا يوجد محرك اهتزاز لمسي على هذه المنصة (الأزرار صامتة)
haptics_light = خفيف
haptics_medium = متوسط
haptics_heavy = قوي
haptics_success = نجاح
haptics_warning = تحذير
haptics_error = خطأ
haptics_selection = تحديد
haptics_last = آخر تشغيل
haptics_none = لم يُشغَّل شيء بعد
haptics_last_played = شُغِّل: { $style }

# --- day-part-prefs ---
nav_prefs = التفضيلات
prefs_caption = احفظ سلسلة نصية عبر عمليات التشغيل باستخدام day-part-prefs.
prefs_placeholder = القيمة المراد تذكّرها
prefs_save = حفظ
prefs_load = قراءة
prefs_clear = مسح
prefs_idle = اكتب قيمة ثم احفظ.
prefs_empty = (لا شيء مخزَّن)
prefs_saved = حُفظ.
prefs_save_failed = فشل الحفظ.
prefs_loaded = قُرئ من التخزين.
prefs_missing = لا شيء مخزَّن بعد.
prefs_cleared = مُسح.
prefs_value_label = القيمة المخزَّنة:

# --- bundled resources (§18.3) ---
nav_resources = الموارد
resources_caption = صورة تُحمَّل بالاسم من مورد مضمَّن، مع قراءات عشوائية لبيانات مضمّنة.
resources_numbers = ‏numbers.bin: { $len } بايت، byte[100] = { $byte }
resources_greeting = ‏greeting.txt: { $text }

# --- day-part-deviceinfo ---
nav_deviceinfo = معلومات الجهاز
deviceinfo_model = الطراز: {$value}
deviceinfo_system = النظام: {$name} {$version}
deviceinfo_simulator = محاكٍ: {$value}
deviceinfo_yes = نعم
deviceinfo_no = لا
deviceinfo_refresh = تحديث

# --- day-piece-activity ---
activity_animating = يتحرّك
activity_on = يدور
activity_off = متوقف

# --- day-piece-searchfield ---
nav_search = البحث
search_placeholder = ابحث عن فاكهة…
search_clear = مسح

# --- day-piece-map ---
nav_map = الخريطة
map_caption = ‏MKMapView أصلي — منصات Apple فقط. انقر إعدادًا مسبقًا لإعادة تمركز الخريطة مباشرة.
map_sf = سان فرانسيسكو
map_nyc = نيويورك

# — tweaks page (docs/tweaks.md) —
nav_tweaks = التوليفات
tweaks_intro = التوليفات المحزَّمة تضبط الودجة الأصلية خلف قطعة مدمجة، حسب حزمة الأدوات. وعلى حزم الأدوات غير المشمولة تكون بلا أثر — القطع أدناه تبدو قياسية ببساطة.
tweaks_stock = قياسي
tweaks_tweaked = مولَّف
tweaks_bezel_title = إطار الزر
tweaks_bezel_caption = ‏day-tweak-button-bezel — ‏AppKit فقط: ثوابت NSBezelStyle على NSButton الحقيقي.
tweaks_selectable_title = تسمية قابلة للتحديد
tweaks_selectable_caption = ‏day-tweak-label-selectable — ‏AppKit وGTK وAndroid: تحديد النص الخاص بالمنصة على تسمية قياسية.
tweaks_selectable_text = يمكن تحديد نص هذه التسمية ونسخه — جرّب ذلك.
tweaks_ticks_title = علامات تدرّج المنزلق
tweaks_ticks_caption = ‏day-tweak-slider-tickmarks — ‏AppKit وGTK وAndroid وQt وWinUI وArkUI: علامات أصلية، مع الالتصاق حيث تدعمه المنصة. المنزلق المولَّف يلتصق؛ والقياسي ينزلق بسلاسة.
tweaks_ref_title = حيوية NativeRef
tweaks_ref_caption = يصل NativeRef إلى المنزلق المولَّف بعد التركيب؛ وعند إزالته يُمسح المرجع بدلًا من أن يبقى معلّقًا.
tweaks_ref_live = المرجع: حي
tweaks_ref_cleared = المرجع: ممسوح

# — merged section pages (design overhaul) —
nav_canvas = اللوحة والأشكال
nav_system = الجهاز والمستشعرات
nav_services = خدمات المنصة
controls_caption = ربط ثنائي الاتجاه: كل عنصر تحكّم إسقاط لإشارة يملكها التطبيق.
controls_basics = الأساسيات
controls_feedback = الاستجابة
canvas_caption = أشكال وتحويلات وإيماءات وقطع الطبقة التركيبية — كلها تُرسم عبر اللوحة.
canvas_gauge = مقياس اللوحة
shapes_interact_hint = اسحب المنزلق للتدوير، وانقر الدائرة لتغيير لونها، واسحب المربع البنفسجي لتحريكه.
system_caption = قطع حالة الجهاز بلا واجهة: البطارية والاتصال ومستشعرات الحركة وهوية الجهاز.
services_caption = قطع «التعامل مع نظام التشغيل» بلا واجهة: الحافظة والتفضيلات والاهتزاز اللمسي ومنتقيات الملفات وHTTP.
subscribe_label = اشتراك

# — data strings localized for the walkthrough locales (option lists, specimen rows) —
chocolate = شوكولاتة
size_small = صغير
size_medium = متوسط
size_large = كبير
fruit_apple = تفاحة
fruit_banana = موزة
fruit_cherry = كرزة
fruit_date = تمرة
fruit_elderberry = بيلسان
list_row = الصف { $n }
text_style_large_title = عنوان كبير
text_style_title = عنوان
text_style_title2 = عنوان 2
text_style_title3 = عنوان 3
text_style_headline = عنوان رئيسي
text_style_subheadline = عنوان فرعي
text_style_body = متن
text_style_callout = تنويه
text_style_footnote = حاشية
text_style_caption = تسمية توضيحية
text_style_caption2 = تسمية توضيحية 2
text_weight_ultralight = رقيق جدًا
text_weight_light = رقيق
text_weight_regular = عادي
text_weight_medium = متوسط
text_weight_semibold = شبه غامق
text_weight_bold = غامق
text_weight_heavy = ثقيل
text_weight_black = أسود
text_bold = نص غامق
text_italic = نص مائل
text_bolditalic = غامق مائل
text_emphasis_label = توكيد
color_red = أحمر
color_green = أخضر
color_blue = أزرق
color_orange = برتقالي

# القوائم والحوارات (صفحة مدموجة)
menus_appmenu_section = قائمة التطبيق
menus_context_section = القائمة السياقية
menus_dialogs_section = الحوارات
modal_result_label = النتيجة

# صفحة الوسائط
media_caption = مشغّل وسائط أصلي — عرض المنصة نفسها، والتحكم عبر المشغّلات.
media_player_section = الفيديو

# أقسام صفحة الموارد
resources_image_section = صورة مضمّنة
resources_modes_note = صورة واحدة بثلاثة أوضاع — الملاءمة تحفظ النسب، والملء يقص، والتمديد يشوّه.
image_mode_fit = ملاءمة
image_mode_fill = ملء
image_mode_stretch = تمديد
resources_data_section = بيانات مضمّنة

# صفحة حول
about_caption = ما هو هذا التطبيق، والمنصة التي يعمل عليها.
about_app_section = هذا التطبيق
about_version = الإصدار
about_toolkit = عدة الأدوات
about_battery = البطارية
history_hint = اضغط + أو − أعلاه وسيظهر كل تغيير هنا.

# صفحة التركيز (docs/focus.md)
nav_focus = التركيز
focus_caption = التركيز ارتباط ثنائي الاتجاه: التغييرات الأصلية تكتب الإشارة، وكتابة الإشارة تنقل التركيز.
focus_group_section = إشارة واحدة، نموذج واحد
focus_group_caption = ثلاثة حقول مرتبطة بإشارة اختيارية واحدة. انقر أو تنقّل بينها فيتبعها المؤشر؛ وزر الإدخال ينتقل إلى الحقل التالي.
focus_name_label = الاسم
focus_email_label = البريد الإلكتروني
focus_city_label = المدينة
focus_current_label = التركيز
focus_next = التركيز التالي
focus_clear = مسح التركيز
focus_bool_section = عنصر واحد، قيمة منطقية واحدة
focus_bool_caption = الحقل نفسه مرتبط بإشارة منطقية — الأزرار تكتبها، والدخول إلى الحقل والخروج منه يكتبها بدوره.
focus_bool_placeholder = التركيز يصل هنا
focus_focus_btn = تركيز
focus_blur_btn = إزالة التركيز
focus_state_label = الحالة
focus_state_on = مركّز
focus_state_off = غير مركّز
focus_probe_section = ما بعد حقول النص
focus_probe_caption = تمنح أدوات سطح المكتب التركيز أيضًا للأزرار والمفاتيح وأشرطة التمرير؛ أما المنصات اللمسية فتخصصه غالبًا لإدخال النص.
focus_probe_toggle = مفتاح
focus_probe_slider = شريط تمرير
focus_probe_button = زر

# HTTP fetch demo (docs/http.md) — the status readout stays raw "<status> <body>" so the
# walkthrough asserts it byte-for-byte in every locale.
http_title = HTTP
http_caption = قطعة day-part-http تمرّ عبر مكدّس HTTP الخاص بالمنصة — وكلاؤه وVPN وTLS.
http_fetch = الجلب من localhost
http_idle = لم يُجلب شيء بعد
http_tier = المكدّس
http_url_placeholder = https://example.com
http_check = فحص
http_checking = يجري الفحص…

# Scrolling page (docs/scroll.md) — programmatic scroll targets.
nav_scrolling = التمرير
scrolling_caption = تمرير برمجي: إشارة Signal تنقل منطقة التمرير إلى حافة أو موضع أو عنصر بعينه.
scroll_to_top = التمرير إلى الأعلى
scroll_to_bottom = التمرير إلى الأسفل
scroll_to_item = التمرير إلى العنصر 100
scrolling_item = العنصر { $n }

# Grid page (docs/grid.md) — grid/grid_row from basics to a stress test.
nav_grid = الشبكة
grid_caption = أعمدة تُقاس حسب المحتوى، وخلايا تمتد عبر الأعمدة، وخلايا مرنة تتقاسم العرض المتبقي
grid_tab_basics = الأساسيات
grid_tab_sizing = الأحجام
grid_tab_spanning = الامتداد
grid_tab_composite = التركيب
grid_tab_stress = اختبار الضغط
grid_basics_caption = يأخذ كل عمود عرض أوسع خلية فيه. لا عروض ثابتة ولا فواصل حشو.
grid_col_name = الاسم
grid_col_wins = الانتصارات
grid_col_points = النقاط
grid_sizing_caption = أعمدة ثابتة وأخرى بحسب المحتوى وأخرى مرنة في شبكة واحدة.
grid_sizing_fixed = ثابت 80 نقطة
grid_sizing_content = حسب المحتوى
grid_sizing_short = قصير
grid_sizing_longer = خلية بمحتوى أطول
grid_spanning_caption = يمكن للخلية أن تمتد عبر عدة أعمدة؛ والعنصر خارج أي صف يمتد عبر الشبكة كلها.
grid_month_title = مخطط الأسبوع
grid_event_focus = فترة تركيز
grid_event_review = مراجعة
grid_composite_caption = الأشكال والشبكة معًا: مجموعات رموز في أعمدة بحسب المحتوى بجانب شريط مدى مرن.
grid_day_n = اليوم { $n }
grid_stress_cells = { $n } صفًا في كل منها 8 خلايا، تُرتَّب كلها فورًا. تحديث خلية واحدة يعيد قياسها وحدها.
grid_stress_add = أضف 50 صفًا
grid_stress_bump = زد الخلية الأولى
