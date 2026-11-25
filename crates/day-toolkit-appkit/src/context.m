// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#import <AppKit/AppKit.h>
#import <objc/runtime.h>

static char popupKey, toastKey;

@interface DayContextPopover : NSPopover <NSPopoverDelegate>
@property uint64_t node;
@property void (*dismissed)(uint64_t);
@end

@implementation DayContextPopover
- (void)popoverDidClose:(NSNotification *)notification {
    if (_dismissed) _dismissed(_node);
}
@end

bool day_appkit_open_popover(NSView *anchor, NSView *panel, double width, double height,
                            uint64_t node, void (*dismissed)(uint64_t)) {
    if (!anchor.window) return false;
    DayContextPopover *popup = [DayContextPopover new];
    popup.node = node;
    popup.dismissed = dismissed;
    popup.delegate = popup;
    popup.behavior = NSPopoverBehaviorTransient;
    NSViewController *controller = [NSViewController new];
    panel.frame = NSMakeRect(0, 0, width, height);
    controller.view = panel;
    popup.contentViewController = controller;
    popup.contentSize = NSMakeSize(width, height);
    objc_setAssociatedObject(panel, &popupKey, popup, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    [popup showRelativeToRect:anchor.bounds ofView:anchor preferredEdge:
        anchor.isFlipped ? NSRectEdgeMaxY : NSRectEdgeMinY];
    return true;
}

void day_appkit_resize_popover(NSView *panel, double width, double height) {
    DayContextPopover *popup = objc_getAssociatedObject(panel, &popupKey);
    popup.contentSize = NSMakeSize(width, height);
}

void day_appkit_close_popover(NSView *panel) {
    DayContextPopover *popup = objc_getAssociatedObject(panel, &popupKey);
    popup.dismissed = NULL;
    [popup close];
    popup.contentViewController = nil;
    objc_setAssociatedObject(panel, &popupKey, nil, OBJC_ASSOCIATION_ASSIGN);
}

@interface DayToastOverlay : NSView
@end
@implementation DayToastOverlay
- (void)hostBoundsChanged:(NSNotification *)notification {
    self.frame = self.superview.bounds;
}
- (void)viewWillMoveToSuperview:(NSView *)parent {
    NSNotificationCenter *center = NSNotificationCenter.defaultCenter;
    if (self.superview)
        [center removeObserver:self name:NSViewBoundsDidChangeNotification object:self.superview];
    [super viewWillMoveToSuperview:parent];
    if (parent) {
        parent.postsBoundsChangedNotifications = YES;
        [center addObserver:self selector:@selector(hostBoundsChanged:)
                       name:NSViewBoundsDidChangeNotification object:parent];
    }
}
- (NSView *)hitTest:(NSPoint)point { return nil; }
@end

void day_appkit_toast(NSView *host, NSString *text) {
    NSView *root = host.window.contentView;
    if (!root) return;
    [objc_getAssociatedObject(root, &toastKey) removeFromSuperview];
    // Day shifts the content view's bounds below the toolbar. Lay out the card in an
    // overlay with its own zero-origin bounds so its bottom edge stays inside the window.
    DayToastOverlay *overlay = [[DayToastOverlay alloc] initWithFrame:root.bounds];
    overlay.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    [root addSubview:overlay positioned:NSWindowAbove relativeTo:nil];
    NSVisualEffectView *card = [NSVisualEffectView new];
    card.material = NSVisualEffectMaterialPopover;
    card.blendingMode = NSVisualEffectBlendingModeWithinWindow;
    card.wantsLayer = YES;
    card.layer.cornerRadius = 8;
    card.translatesAutoresizingMaskIntoConstraints = NO;
    NSTextField *label = [NSTextField wrappingLabelWithString:text];
    label.translatesAutoresizingMaskIntoConstraints = NO;
    [card addSubview:label];
    [overlay addSubview:card];
    [NSLayoutConstraint activateConstraints:@[
        [label.leadingAnchor constraintEqualToAnchor:card.leadingAnchor constant:12],
        [label.trailingAnchor constraintEqualToAnchor:card.trailingAnchor constant:-12],
        [label.topAnchor constraintEqualToAnchor:card.topAnchor constant:8],
        [label.bottomAnchor constraintEqualToAnchor:card.bottomAnchor constant:-8],
        [card.widthAnchor constraintLessThanOrEqualToAnchor:overlay.widthAnchor constant:-24],
        [card.centerXAnchor constraintEqualToAnchor:overlay.centerXAnchor],
        [card.bottomAnchor constraintEqualToAnchor:overlay.bottomAnchor constant:-16]
    ]];
    objc_setAssociatedObject(root, &toastKey, overlay, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    __weak NSView *owner = root;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC), dispatch_get_main_queue(), ^{
        [overlay removeFromSuperview];
        if (owner && objc_getAssociatedObject(owner, &toastKey) == overlay)
            objc_setAssociatedObject(owner, &toastKey, nil, OBJC_ASSOCIATION_ASSIGN);
    });
}
