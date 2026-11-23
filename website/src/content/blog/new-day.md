---
title: A new Day for app development in Rust
description: Build cross-platform Rust apps with native widgets and a shared toolchain for building, testing, packaging, and distribution.
pubDate: 2026-11-23
author: Marc Prud'hommeaux
---

Day is a cross-platform application framework for Rust that uses each platform’s native UI toolkit. Its declarative, reactive API lets you share application code, while the `day` toolchain handles building, testing, packaging, and distribution.

Day is a "batteries-included" solution, containing everything you need to create the highest-quality and most efficient applications possible, covering every aspect of the software delivery lifecycle, and includes integration with GitHub for continuous integration, UI testing, localized screenshot generation, and the generation of a customizable landing page.

## The rush to Rust

Rust is all the rage these days. Once confined to only the most elite galaxy-brain developers who could wrangle the syntax and spar with borrow checkers, Rust's amenability to agentic development has greatly flattened the previously steep learning curve for this amazing language, and thereby broadened the audience that is able to take advantage of it. These days, anyone can gain entry to the rarefied ranks of the Rust intelligentsia.

And yet, while the Rust ecosystem itself is replete with APIs for creating graphical user interfaces, they all somehow miss the mark. The popular community [Are we GUI Yet?](https://areweguiyet.com) survey site summarizes the state of affairs thusly:
 
<figure class="source-quote">
  <span class="source-quote-mark" aria-hidden="true">“</span>
  <blockquote cite="https://areweguiyet.com">
    <p>Rust's expressiveness and high level abstractions make it ideal for building intricate and complex user interfaces. Unfortunately, there is little consensus on what the best abstractions are. There are a number of bindings available today to existing frameworks, but those looking for a mature, easy to use, and completely Rust-based solution will most likely find themselves out of luck. Current approaches to building GUIs in Rust include interfacing with Electron and building GUIs with HTML (or a framework on top of it), while others have taken to using graphics APIs and various wrappers to emulate classical widgets.</p>
  </blockquote>
  <figcaption><cite><a href="https://areweguiyet.com">Are we GUI Yet?</a></cite></figcaption>
</figure>

So basically, your choices are to paint mimicked widgets onto a canvas or slap a web renderer around an HTML page or JavaScript canvas and bridge it through to a separate Rust process. These present the exact same set of compromises and unsatisfactory user experiences as the dominant cross-platform frameworks for the other languages, such as Flutter and Electron. Being written in Rust doesn't somehow excuse these deficiencies.

This past weekend, a timely post titled [**Advice for the next 100 Rust GUIs**](https://linebender.org/blog/next-100-guis/) discusses this state of affairs and goes on to lament that — despite the plethora of attempts thus far — we haven't gotten very close to solving these shortcomings. In the [_Native is complicated_](https://linebender.org/blog/next-100-guis/#native-is-complicated) section, the author writes:

<figure class="source-quote">
  <span class="source-quote-mark" aria-hidden="true">“</span>
  <blockquote cite="https://linebender.org/blog/next-100-guis/#native-is-complicated">
    <p>Platforms continue to invest, and there are many fine details that are important for an app to look and feel native. One very notable example is Liquid Glass on iOS – if you use UIKit or SwiftUI, you can get that with little effort, but apps built with cross-platform toolkits won’t quite seem right. In addition, making gesture recognition, scroll dynamics, and IME 100% match the platform are not yet fully solved problems.</p>
  </blockquote>
  <figcaption>Raph Levien · <cite><a href="https://linebender.org/blog/next-100-guis/#native-is-complicated">Advice for the next 100 Rust GUIs</a></cite> · Linebender</figcaption>
</figure>

Day solves these problems by taking a different approach. Rather than pixel-pushing or web-jockeying, it drives the platform's native widgets, screens, and navigation paradigms that are used throughout the rest of the host OS. Day creates apps that are indistinguishable from the first-party user interfaces that the platform vendors themselves (Apple, Google, Microsoft, etc.) recommend and use for most of their own applications.

You _could_, of course, do all this manually yourself: implement a platform-native Windows application with WinUI in C#, macOS with AppKit in Objective-C, Android in Java, iOS in Swift, Linux GTK in C, Qt in C++, and web in JavaScript. That would _also_ achieve the same premium-quality and high-performance experience that users want. But that's precisely what Day abstracts away: you can create the logical interface with an intuitive declarative API, and Day deals with all the messy details of the underlying platform's nuts and bolts, while still exposing the ability to dip down into the platform-specific framework for fine-grained customization.

## The Dawn of a Revolution

I've been building apps for decades. Back in the 1990s, I was an eager drinker of the Java "write once run anywhere" kool-aid, and briefly thought that Swing was the solution for the ages. More recently, I considered Swift and SwiftUI the best path forward — at least as far as bridging the iOS/Android divide — which is why I made [Skip](https://skip.dev).

After all those years of chasing the cross-platform holy grail, I've come to a somewhat counterintuitive conclusion: the best way to make a great app that runs everywhere is to stop trying to make it look the same everywhere. That's the core idea behind Day. A single codebase can define the application's structure and behavior, while at the same time respecting the platform conventions and leveraging all the benefits of using the first-party SDKs: cohesiveness, performance, accessibility, and ongoing platform improvements inherited through native controls.

To be sure, Day is a new framework, but there are already plenty of apps you can download and try right now to see for yourself. The [Day Showcase](/showcase/) app gives an overview of the capabilities of the technology. Our [gallery](/gallery/) presents a variety of real-world applications, including mobile versions that are already shipping to the Apple App Store and Google Play Store: an [ebook reader](/gallery/Stanza-Redux/), a [games app](/gallery/Games-Fair/), and an [RSS reader](/gallery/Fleet-Reader/). Try them out for yourself and see what a difference using the true platform-native toolkit makes.


<figure style="margin: 2rem 0;">
  <video controls muted playsinline preload="none" width="1920" height="1080" poster="https://media.daybrite.cloud/reels/candidate-07-full-4k/poster.jpg" aria-labelledby="showcase-video-caption" style="display: block; width: 100%; height: auto; border-radius: var(--radius-lg); background: #000;">
    <source src="https://media.daybrite.cloud/reels/candidate-07-full-4k/master.m3u8" type="application/vnd.apple.mpegurl">
    <source src="https://media.daybrite.cloud/reels/candidate-07-full-4k/showreel-1080p.mp4" type="video/mp4">
  </video>
  <figcaption id="showcase-video-caption">Day Showcase running on iPhone, Android, HarmonyOS, macOS, Linux, and Windows. <a href="/showcase">Try the Showcase app.</a></figcaption>
</figure>

## Rise and Shine

Until very recently, convincing you to try out a new app development framework would have required cajoling and enticements. No longer. It has become a well-established fact that agents excel at writing and maintaining Rust codebases, and they can whip up a proof of concept in a jiffy.

To see Day in action, simply [fire up](https://daybrite.dev/docs/tutorials/ai-assisted-app/) your agent of choice (Claude Code and Codex work especially well) and run something like:

<div class="terminal-prompt">
  <div class="terminal-prompt-bar" aria-hidden="true"><span></span><span></span><span></span></div>
  <div class="terminal-prompt-body">
    <span class="terminal-prompt-sigil" aria-hidden="true">❯</span>
    <p>Using the guidance at https://daybrite.dev/llms.txt build and launch a Day app for [<em>macOS, Windows, Linux, iOS, Android, and/or web</em>] that [<em>does whatever you want</em>].<span class="terminal-prompt-cursor" aria-hidden="true"></span></p>
  </div>
</div>

… and then let it whirr for a bit. Once it comes up, you will experience firsthand the beauty and efficiency of a genuinely platform-native application. Be sure to have your app take advantage of the [daybrite ecosystem](https://github.com/daybrite/) of optional packages for Day: [media](https://github.com/daybrite/day-piece-media), [bluetooth](https://github.com/daybrite/day-part-bluetooth), [maps](https://github.com/daybrite/day-piece-map), [charts](https://github.com/daybrite/day-piece-charts), [web views](https://github.com/daybrite/day-piece-webview), and much more.

And for extra credit, push it to a GitHub repo with [Actions](https://daybrite.dev/docs/how-to/github-actions/) and [Pages](https://daybrite.dev/docs/how-to/websites/) enabled, let it run through the build, and post your work to the daybrite [show-and-tell forum](https://github.com/orgs/daybrite/discussions/categories/show-and-tell). Follow up with automated [app store distribution](https://daybrite.dev/docs/how-to/app-store-submissions/) and your project is done and dusted.

A new Day is here. It's time to rise and shine.
