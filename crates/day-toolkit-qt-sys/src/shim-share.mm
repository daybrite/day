// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
#import <Cocoa/Cocoa.h>
extern "C" bool day_qt_share_url(const char *url, const char *) {
    @autoreleasepool {
        NSURL *item = [NSURL URLWithString:[NSString stringWithUTF8String:url]];
        NSView *view = NSApp.keyWindow.contentView;
        if (!item || !view) return false;
        static NSSharingServicePicker *picker = nil;
        [picker release];
        picker = [[NSSharingServicePicker alloc] initWithItems:@[item]];
        CGFloat y = view.isFlipped ? 30.0 : NSHeight(view.bounds) - 30.0;
        [picker showRelativeToRect:NSMakeRect(NSWidth(view.bounds) - 30.0, y, 1, 1)
                          ofView:view preferredEdge:NSMaxYEdge];
        return true;
    }
}
