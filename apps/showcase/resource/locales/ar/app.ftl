app-title = عرض Day
counter-value = { $count ->
    [zero] لا نقرات
    [one] نقرة واحدة
    [two] نقرتان
    [few] { $count } نقرات
   *[other] { $count } نقرة
}
decrement = −
increment = +
name-placeholder = اسمك
greeting = مرحبًا، { $name }!
volume-label = مستوى الصوت
progress-label = التقدّم
busy-label = مشغول
flavor-label = النكهة
history-title = السجلّ
history-entry = أصبح العدّاد { $value }
nav-controls = عناصر التحكّم
nav-menus = القوائم
nav-text = النص
nav-battery = البطارية
nav-sensors = المستشعرات
nav-clipboard = الحافظة
nav-network = الشبكة
nav-media = الوسائط
nav-pickers = المنتقيات
nav-compose = التركيب
nav-files = الملفات
nav-tabs = علامات التبويب
nav-stack = المكدّس
nav-list = القائمة
nav-webview = عرض الويب
nav-lottie = Lottie
nav-about = حول

shapes-kinds = الأنواع
shapes-transform = التحويل
shapes-angle = الزاوية

picker-shared-caption = الأنماط الثلاثة مرتبطة بإشارة اختيار واحدة — غيّر أحدها وسيتبعه الآخران.
picker-selected = المحدد
picker-segmented = مقسّم
picker-menu = قائمة
picker-inline = مضمّن

compose-caption = قطع تركيبية بحتة — بلا شيفرة أصلية ولا ميزات cargo، وتعمل على كل واجهة خلفية مباشرة.
compose-rating-label = تقييم بالنجوم
compose-rating-count = النجوم المحدّدة:
compose-rating-placeholder = ١–٥
compose-card-title = سطح قابل لإعادة الاستخدام
compose-card-body = حشوة + خلفية + زوايا مستديرة، تُطبَّق كمُعدِّل.
compose-plain-btn = عادي
compose-styled-btn = ممتلئ
compose-env-value = ملوَّن باللون المميّز المُمرَّر
list-add = أضف ١٠٠
list-caption = { $count } صف — تُبنى الخلايا المرئية فقط

webview-url-hint = أدخل عنوان URL
webview-go = انتقال
webview-back = رجوع
webview-forward = تقدّم
webview-stop = إيقاف
webview-reload = إعادة تحميل

lottie-caption = رسم Lottie متحرّك أصلي، مضمَّن كملف JSON‏ (lottie-ios / lottie-android)
lottie-speed = السرعة
stack-root-body = مكدّس دفع/سحب حقيقي. مساره إشارة يملكها التطبيق.
stack-push = ادفع صفحة تفاصيل
stack-detail-title = المستوى { $depth }
stack-detail-body = دُفع إلى المسار. زر الرجوع الأصلي يكتب السحب مرة أخرى.
stack-item-title = العنصر { $id }
stack-link-42 = افتح item-42 مع تلميح (مسار مطلق)
stack-param-hint = فُتح مع تلميح: {$hint}
tab-one = نظرة عامة
tab-two = التفاصيل
tab-three = الإعدادات
tab-one-body = علامة تبويب النظرة العامة. تحتفظ كل علامة تبويب بحالتها الخاصة.
tab-two-body = علامة تبويب التفاصيل، تُحدَّد بمفتاح مسارها.
tab-three-body = علامة تبويب الإعدادات. الروابط العميقة وdayscript يحدّدان علامات التبويب بالمفتاح.
about-text = تطبيق أصلي متعدد المنصات مبني بـ day.
nav-modals = النوافذ المشروطة
modal-alert = إظهار تنبيه
modal-confirm = تأكيد
modal-delete = حذف…
modal-sheet = اختر نكهة
modal-prompt = أدخل الاسم
alert-title = إشعار
alert-body = حُفظت تغييراتك.
ok = حسنًا
confirm-title = إنهاء؟
confirm-body = هل أنت متأكد أنك تريد الإنهاء؟
delete-title = حذف العنصر؟
delete-body = لا يمكن التراجع عن هذا.
delete = حذف
flavor-title = اختر نكهة
cancel = إلغاء
vanilla = فانيليا
pistachio = فستق

# Files playground (docs/files.md)
files-caption = منتقيات فتح/حفظ الملفات الأصلية. «فتح» يقرأ ملفًا نصيًا إلى المحرّر؛ و«حفظ» يكتبه مرة أخرى.
files-placeholder = اكتب شيئًا لحفظه…
files-open = فتح ملف…
files-save = حفظ الملف…
files-opened = فُتح { $name }

# Battery playground (docs/battery.md)
battery-refresh = قراءة بطارية الجهاز
battery-level = المستوى
battery-charging = يُشحن
battery-reading = البطارية: { $percent } · { $state }
battery-reading-none = البطارية: لا توجد واجهة برمجة للبطارية على هذه المنصة

# Sensors playground (docs/sensors.md)
sensors-refresh = قراءة المستشعرات
sensor-accelerometer = مقياس التسارع
sensor-gyroscope = الجيروسكوب
sensor-magnetometer = مقياس المغناطيسية
sensor-reading = x { $x } · y { $y } · z { $z } { $unit }
sensor-waiting = في انتظار أول عيّنة…
sensor-unavailable = غير متوفر على هذا الجهاز

# Clipboard playground (docs/clipboard.md)
clipboard-caption = قطعة day-part-clipboard تقرأ الحافظة النظامية وتكتب فيها بشكل أصلي.
clipboard-placeholder = اكتب شيئًا لنسخه
clipboard-copy = نسخ
clipboard-paste = لصق
clipboard-idle = الحافظة لم تُمسّ
clipboard-copied = نُسخ إلى الحافظة النظامية
clipboard-copy-failed = فشل النسخ (لا توجد واجهة حافظة هنا)
clipboard-pasted = لُصق من الحافظة النظامية
clipboard-empty = الحافظة فارغة (أو غير قابلة للقراءة في الخلفية)

# Network playground (docs/network.md)
network-refresh = قراءة الشبكة
network-reading-online = متصل · { $kind } · مقنَّن: { $expensive }
network-reading-offline = غير متصل
network-reading-none = لا توجد واجهة اتصال على هذه المنصة

# Media playground (docs/media.md)
media-play = تشغيل
media-pause = إيقاف مؤقت
media-load = تحميل

# Text playground (typography)
text-caption = الأنماط الدلالية تُطابق أنماط النص الأصلية للمنصة وتكبير نص إمكانية الوصول.
text-styles-header = الأنماط
text-weights-header = الأوزان
text-styling-header = عريض ومائل
text-colors-header = اللون
text-custom-header = أحجام مخصّصة
text-custom-note = ‏Font.System(pt) — لا يزال يُكبَّر وفق حجم نص إمكانية الوصول (الخط الديناميكي / مقياس الخط).
text-fonts-header = الخطوط المضمّنة
text-fonts-note = ‏Font.Custom("Family", pt) — ملفات من مجلد resource/fonts/ في التطبيق، يضمّها day build وتُحلّ باسم العائلة على كل منصة.

# Menus playground
menus-caption = قوائم أصلية — شريط قوائم التطبيق وقوائم السياق لكل قطعة — مع قوائم فرعية متداخلة واختصارات لوحة المفاتيح وأوامر التحرير القياسية.
menus-last = آخر إجراء:
menus-lifecycle = آخر مرحلة دورة حياة:
menus-context-hint = قائمة السياق
menus-target = انقر بزر الفأرة الأيمن هنا (ضغطة مطوّلة على الجوال) لقائمة سياق
menus-shortcut-hint = اختصارات لوحة المفاتيح (⌘/Ctrl + مفتاح) تظهر في شريط القوائم وتعمل أثناء تركيز التطبيق — مثل جديد (N)، حفظ (S)، إعادة تحميل (R)، حفظ باسم (⇧S).

# --- day-part-haptics ---
nav-haptics = الاهتزاز اللمسي
haptics-supported-yes = محرك الاهتزاز اللمسي متوفر على هذه المنصة
haptics-supported-no = لا يوجد محرك اهتزاز لمسي على هذه المنصة (الأزرار صامتة)
haptics-light = خفيف
haptics-medium = متوسط
haptics-heavy = قوي
haptics-success = نجاح
haptics-warning = تحذير
haptics-error = خطأ
haptics-selection = تحديد
haptics-last = آخر تشغيل
haptics-none = لم يُشغَّل شيء بعد
haptics-last-played = شُغِّل: { $style }

# --- day-part-prefs ---
nav-prefs = التفضيلات
prefs-caption = احفظ سلسلة نصية عبر عمليات التشغيل باستخدام day-part-prefs.
prefs-placeholder = القيمة المراد تذكّرها
prefs-save = حفظ
prefs-load = قراءة
prefs-clear = مسح
prefs-idle = اكتب قيمة ثم احفظ.
prefs-empty = (لا شيء مخزَّن)
prefs-saved = حُفظ.
prefs-save-failed = فشل الحفظ.
prefs-loaded = قُرئ من التخزين.
prefs-missing = لا شيء مخزَّن بعد.
prefs-cleared = مُسح.
prefs-value-label = القيمة المخزَّنة:

# --- bundled resources (§18.3) ---
nav-resources = الموارد
resources-caption = صورة تُحمَّل بالاسم من مورد مضمَّن، مع قراءات عشوائية لبيانات مضمّنة.
resources-numbers = ‏numbers.bin: { $len } بايت، byte[100] = { $byte }
resources-greeting = ‏greeting.txt: { $text }

# --- day-part-deviceinfo ---
nav-deviceinfo = معلومات الجهاز
deviceinfo-model = الطراز: {$value}
deviceinfo-system = النظام: {$name} {$version}
deviceinfo-simulator = محاكٍ: {$value}
deviceinfo-yes = نعم
deviceinfo-no = لا
deviceinfo-refresh = تحديث

# --- day-piece-activity ---
activity-animating = يتحرّك
activity-on = يدور
activity-off = متوقف

# --- day-piece-searchfield ---
nav-search = البحث
search-placeholder = ابحث عن فاكهة…
search-clear = مسح

# --- day-piece-map ---
nav-map = الخريطة
map-caption = ‏MKMapView أصلي — منصات Apple فقط. انقر إعدادًا مسبقًا لإعادة تمركز الخريطة مباشرة.
map-sf = سان فرانسيسكو
map-nyc = نيويورك

# — tweaks page (docs/tweaks.md) —
nav-tweaks = التوليفات
tweaks-intro = التوليفات المحزَّمة تضبط الودجة الأصلية خلف قطعة مدمجة، حسب حزمة الأدوات. وعلى حزم الأدوات غير المشمولة تكون بلا أثر — القطع أدناه تبدو قياسية ببساطة.
tweaks-stock = قياسي
tweaks-tweaked = مولَّف
tweaks-bezel-title = إطار الزر
tweaks-bezel-caption = ‏day-tweak-button-bezel — ‏AppKit فقط: ثوابت NSBezelStyle على NSButton الحقيقي.
tweaks-selectable-title = تسمية قابلة للتحديد
tweaks-selectable-caption = ‏day-tweak-label-selectable — ‏AppKit وGTK وAndroid: تحديد النص الخاص بالمنصة على تسمية قياسية.
tweaks-selectable-text = يمكن تحديد نص هذه التسمية ونسخه — جرّب ذلك.
tweaks-ticks-title = علامات تدرّج المنزلق
tweaks-ticks-caption = ‏day-tweak-slider-tickmarks — ‏AppKit وGTK وAndroid وQt وWinUI وArkUI: علامات أصلية، مع الالتصاق حيث تدعمه المنصة. المنزلق المولَّف يلتصق؛ والقياسي ينزلق بسلاسة.
tweaks-ref-title = حيوية NativeRef
tweaks-ref-caption = يصل NativeRef إلى المنزلق المولَّف بعد التركيب؛ وعند إزالته يُمسح المرجع بدلًا من أن يبقى معلّقًا.
tweaks-ref-live = المرجع: حي
tweaks-ref-cleared = المرجع: ممسوح

# — merged section pages (design overhaul) —
nav-canvas = اللوحة والأشكال
nav-system = الجهاز والمستشعرات
nav-services = خدمات المنصة
controls-caption = ربط ثنائي الاتجاه: كل عنصر تحكّم إسقاط لإشارة يملكها التطبيق.
controls-basics = الأساسيات
controls-feedback = الاستجابة
canvas-caption = أشكال وتحويلات وإيماءات وقطع الطبقة التركيبية — كلها تُرسم عبر اللوحة.
canvas-gauge = مقياس اللوحة
shapes-interact-hint = اسحب المنزلق للتدوير، وانقر الدائرة لتغيير لونها، واسحب المربع البنفسجي لتحريكه.
system-caption = قطع حالة الجهاز بلا واجهة: البطارية والاتصال ومستشعرات الحركة وهوية الجهاز.
services-caption = قطع «التعامل مع نظام التشغيل» بلا واجهة: الحافظة والتفضيلات والاهتزاز اللمسي ومنتقيات الملفات.
subscribe-label = اشتراك

# — data strings localized for the walkthrough locales (option lists, specimen rows) —
chocolate = شوكولاتة
size-small = صغير
size-medium = متوسط
size-large = كبير
fruit-apple = تفاحة
fruit-banana = موزة
fruit-cherry = كرزة
fruit-date = تمرة
fruit-elderberry = بيلسان
list-row = الصف { $n }
text-style-large-title = عنوان كبير
text-style-title = عنوان
text-style-title2 = عنوان 2
text-style-title3 = عنوان 3
text-style-headline = عنوان رئيسي
text-style-subheadline = عنوان فرعي
text-style-body = متن
text-style-callout = تنويه
text-style-footnote = حاشية
text-style-caption = تسمية توضيحية
text-style-caption2 = تسمية توضيحية 2
text-weight-ultralight = رقيق جدًا
text-weight-light = رقيق
text-weight-regular = عادي
text-weight-medium = متوسط
text-weight-semibold = شبه غامق
text-weight-bold = غامق
text-weight-heavy = ثقيل
text-weight-black = أسود
text-bold = نص غامق
text-italic = نص مائل
text-bolditalic = غامق مائل
text-emphasis-label = توكيد
color-red = أحمر
color-green = أخضر
color-blue = أزرق
color-orange = برتقالي
