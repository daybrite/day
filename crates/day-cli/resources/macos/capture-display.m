// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The capture display for scripted macos-appkit runs (website docs "dayscript", "Capture
// size"). Embedded in day-cli (screenshot.rs), compiled once with the host's clang, and run as
//   day-capture-display <width> <height> <scale> [force]
// where width and height are the window in points and scale is what the capture is rendered at.
//
// day-appkit captures by reading the window server's composited pixels back, which is the only
// capture that shows macOS's own materials, so a capture has the scale of the display the
// window is on. A CI runner's display is 1x: a 1280x800-point window comes back 1280x800
// pixels where a laptop produces 2560x1600. When no attached display has the scale the run
// asked for, this creates a HiDPI virtual display the window server composites at that scale
// and prints its id; the CLI passes the id to the app as DAY_WINDOW_SCREEN, and the app opens
// its window there. The virtual display sits beside the real ones and rearranges nothing.
//
// Output, one line on stdout:
//   native              a display with the scale is already attached; nothing was created
//   display <id>        the virtual display's CGDirectDisplayID; alive until stdin closes
//   unavailable <why>   (exit 1) no virtual display could be made; the run captures at 1x
//
// CGVirtualDisplay is CoreGraphics API that ships without a header. It is declared here and
// used only by this development tool, never linked into an app.
#import <Cocoa/Cocoa.h>

@interface CGVirtualDisplayDescriptor : NSObject
@property(retain) dispatch_queue_t queue;
@property(retain) NSString *name;
@property uint32_t maxPixelsWide;
@property uint32_t maxPixelsHigh;
@property CGSize sizeInMillimeters;
@property uint32_t productID;
@property uint32_t vendorID;
@property uint32_t serialNum;
@end

@interface CGVirtualDisplayMode : NSObject
- (instancetype)initWithWidth:(NSUInteger)width height:(NSUInteger)height refreshRate:(double)rate;
@end

@interface CGVirtualDisplaySettings : NSObject
@property uint32_t hiDPI;
@property(retain) NSArray *modes;
@end

@interface CGVirtualDisplay : NSObject
- (instancetype)initWithDescriptor:(CGVirtualDisplayDescriptor *)descriptor;
- (BOOL)applySettings:(CGVirtualDisplaySettings *)settings;
@property(readonly) CGDirectDisplayID displayID;
@end

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc < 4) {
            fprintf(stderr, "usage: day-capture-display <width> <height> <scale> [force]\n");
            return 2;
        }
        double width = atof(argv[1]), height = atof(argv[2]), scale = atof(argv[3]);
        BOOL force = argc > 4 && strcmp(argv[4], "force") == 0;
        // No NSApplication here: a process that has one never sees its virtual display come
        // online (measured on macOS 26), and NSScreen answers without it.

        // A window has to fit under the menu bar with its title bar on, so ask for some room.
        double need_w = width, need_h = height + 80;
        if (!force) {
            for (NSScreen *screen in NSScreen.screens) {
                NSSize visible = screen.visibleFrame.size;
                if (screen.backingScaleFactor >= scale && visible.width >= need_w &&
                    visible.height >= need_h) {
                    printf("native\n");
                    return 0;
                }
            }
        }
        if (scale != 1.0 && scale != 2.0) {
            printf("unavailable a virtual display renders at 1x or 2x, not %gx\n", scale);
            return 1;
        }
        if (!NSClassFromString(@"CGVirtualDisplay")) {
            printf("unavailable this macOS has no CGVirtualDisplay\n");
            return 1;
        }

        NSUInteger mode_w = (NSUInteger)MAX(1920.0, need_w + 160.0);
        NSUInteger mode_h = (NSUInteger)MAX(1200.0, need_h + 160.0);
        CGVirtualDisplayDescriptor *descriptor = [CGVirtualDisplayDescriptor new];
        descriptor.queue = dispatch_get_main_queue();
        descriptor.name = @"Day Capture";
        descriptor.maxPixelsWide = (uint32_t)(mode_w * 2);
        descriptor.maxPixelsHigh = (uint32_t)(mode_h * 2);
        // About 140 points per inch, a laptop panel, so nothing reads the display as a TV.
        descriptor.sizeInMillimeters = CGSizeMake(mode_w * 25.4 / 140.0, mode_h * 25.4 / 140.0);
        descriptor.productID = 0x0DA7;
        descriptor.vendorID = 0x0DA7;
        descriptor.serialNum = 1;
        CGVirtualDisplay *display = [[CGVirtualDisplay alloc] initWithDescriptor:descriptor];
        CGVirtualDisplaySettings *settings = [CGVirtualDisplaySettings new];
        settings.hiDPI = scale == 2.0 ? 1 : 0;
        settings.modes = @[ [[CGVirtualDisplayMode alloc] initWithWidth:mode_w
                                                                  height:mode_h
                                                             refreshRate:60] ];
        if (!display || ![display applySettings:settings] || display.displayID == 0) {
            printf("unavailable the window server refused the virtual display\n");
            return 1;
        }

        // The display comes online on a later run-loop turn. Wait until CoreGraphics reports it
        // online in a mode with the scale, so the app never opens onto a display that is not
        // there yet. (CoreGraphics rather than NSScreen: this process runs no event loop, and
        // NSScreen only refreshes its list from one.)
        CGDirectDisplayID wanted = display.displayID;
        BOOL up = NO;
        for (int i = 0; i < 100 && !up; i++) {
            [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.1]];
            CGDisplayModeRef mode = CGDisplayCopyDisplayMode(wanted);
            if (mode) {
                size_t points = CGDisplayModeGetWidth(mode), pixels = CGDisplayModeGetPixelWidth(mode);
                up = CGDisplayIsOnline(wanted) && points > 0 && (double)pixels / points >= scale;
                CGDisplayModeRelease(mode);
            }
        }
        if (!up) {
            printf("unavailable the virtual display never came up at %gx\n", scale);
            return 1;
        }
        printf("display %u\n", wanted);
        fflush(stdout);

        // Alive exactly as long as the `day` that started this: its end of the pipe closes when
        // it exits, however it exits, and the display goes with this process.
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
            char buffer[64];
            while (read(STDIN_FILENO, buffer, sizeof buffer) > 0) {
            }
            exit(0);
        });
        [[NSRunLoop currentRunLoop] run];
    }
    return 0;
}
