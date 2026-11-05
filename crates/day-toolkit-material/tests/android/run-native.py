#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Run native menu lifecycle tests inside an installed, isolated debug Day app.

Requires its compiled Java classes, an Android SDK, JAVA_HOME, and the same debug
keystore used to sign that app. Stops the target app while instrumentation runs;
app data is preserved. The caller can launch it again after the test.
"""
import argparse, os, pathlib, subprocess, tempfile, zipfile
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--serial',required=True)
p.add_argument('--package',required=True)
p.add_argument('--classes',required=True,type=pathlib.Path)
p.add_argument('--sdk',type=pathlib.Path,default=os.environ.get('ANDROID_HOME'))
p.add_argument('--keystore',type=pathlib.Path,default=pathlib.Path.home()/'.android/debug.keystore')
a=p.parse_args()
if not a.sdk:p.error('--sdk or ANDROID_HOME required')
def run(*cmd,**kw):return subprocess.run([str(x) for x in cmd],check=True,**kw)
build=sorted((a.sdk/'build-tools').iterdir(),key=lambda x:tuple(int(n) for n in x.name.split('.')))[-1]
android=a.sdk/'platforms/android-36/android.jar'
java=pathlib.Path(os.environ['JAVA_HOME'])/'bin'
source=pathlib.Path(__file__).with_name('ContextMenuInstrumentation.java')
with tempfile.TemporaryDirectory(prefix='day-context-menu-') as tmp:
    work=pathlib.Path(tmp)
    classes=work/'classes';classes.mkdir()
    run(java/'javac','-source','8','-target','8','-Xlint:-options','-cp',str(android)+os.pathsep+str(a.classes),'-d',classes,source)
    dex=work/'dex';dex.mkdir()
    run(build/'d8','--lib',android,'--classpath',a.classes,'--min-api','26','--output',dex,*classes.rglob('*.class'))
    manifest=work/'AndroidManifest.xml'
    manifest.write_text(f'''<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="dev.daybrite.day.contextmenutest">
    <uses-sdk android:minSdkVersion="26" android:targetSdkVersion="36"/>
    <application android:debuggable="true"/>
    <instrumentation android:name="dev.daybrite.day.bridge.ContextMenuInstrumentation" android:targetPackage="{a.package}"/>
</manifest>''')
    apk=work/'test.apk'
    run(build/'aapt2','link','-I',android,'--manifest',manifest,'-o',apk)
    with zipfile.ZipFile(apk,'a') as archive:
        for item in dex.glob('*.dex'):archive.write(item,item.name)
    signed=work/'signed.apk'
    run(build/'apksigner','sign','--ks',a.keystore,'--ks-pass','pass:android','--out',signed,apk)
    adb=[a.sdk/'platform-tools/adb','-s',a.serial]
    run(*adb,'install','-r','-t',signed)
    try:
        result=run(*adb,'shell','am','instrument','-w','dev.daybrite.day.contextmenutest/dev.daybrite.day.bridge.ContextMenuInstrumentation',capture_output=True,text=True)
        print(result.stdout)
        if 'PASS Android native context menus:' not in result.stdout:raise SystemExit(1)
    finally:
        run(*adb,'uninstall','dev.daybrite.day.contextmenutest')
