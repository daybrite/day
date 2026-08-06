# The Rust side reaches DayLocalNotify by name through JNI (dcall_static), and Android instantiates
# the two receivers by the name in the merged manifest. R8 must not rename or strip either.
-keep class dev.daybrite.day.notify.** { *; }
