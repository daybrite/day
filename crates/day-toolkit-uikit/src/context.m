// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#import <UIKit/UIKit.h>
#import <objc/runtime.h>

static char popupKey, tooltipKey, toastKey;

@interface DayContextController : UIViewController <UIPopoverPresentationControllerDelegate>
@property uint64_t node;
@property void (*dismissed)(uint64_t);
@end

@implementation DayContextController
- (UIModalPresentationStyle)adaptivePresentationStyleForPresentationController:(UIPresentationController *)controller
                                                              traitCollection:(UITraitCollection *)traits {
    return UIModalPresentationNone;
}
- (void)presentationControllerDidDismiss:(UIPresentationController *)presentationController {
    if (_dismissed) _dismissed(_node);
}
@end

bool day_uikit_open_popover(UIView *anchor, UIView *panel, double width, double height,
                           uint64_t node, void (*dismissed)(uint64_t)) {
    UIViewController *parent = anchor.window.rootViewController;
    while (parent.presentedViewController) parent = parent.presentedViewController;
    if (!parent || parent.isBeingDismissed) return false;
    DayContextController *popup = [DayContextController new];
    popup.node = node;
    popup.dismissed = dismissed;
    popup.modalPresentationStyle = UIModalPresentationPopover;
    popup.view = panel;
    panel.backgroundColor = UIColor.secondarySystemBackgroundColor;
    popup.preferredContentSize = CGSizeMake(width, height);
    popup.popoverPresentationController.sourceView = anchor;
    popup.popoverPresentationController.sourceRect = anchor.bounds;
    popup.popoverPresentationController.delegate = popup;
    objc_setAssociatedObject(panel, &popupKey, popup, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    [parent presentViewController:popup animated:NO completion:nil];
    return true;
}

void day_uikit_close_popover(UIView *panel) {
    DayContextController *popup = objc_getAssociatedObject(panel, &popupKey);
    popup.dismissed = NULL;
    if (popup.presentingViewController) [popup dismissViewControllerAnimated:NO completion:nil];
    objc_setAssociatedObject(panel, &popupKey, nil, OBJC_ASSOCIATION_ASSIGN);
}

void day_uikit_tooltip(UIView *anchor, NSString *text) {
    if (@available(iOS 15.0, *)) {
        UIToolTipInteraction *tip = objc_getAssociatedObject(anchor, &tooltipKey);
        if (!text.length) {
            if (tip) [anchor removeInteraction:tip];
            objc_setAssociatedObject(anchor, &tooltipKey, nil, OBJC_ASSOCIATION_ASSIGN);
        } else if (tip) {
            tip.defaultToolTip = text;
        } else {
            tip = [[UIToolTipInteraction alloc] initWithDefaultToolTip:text];
            [anchor addInteraction:tip];
            objc_setAssociatedObject(anchor, &tooltipKey, tip, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        }
    }
}

void day_uikit_toast(UIView *host, NSString *text) {
    UIWindow *window = host.window;
    if (!window) return;
    [objc_getAssociatedObject(window, &toastKey) removeFromSuperview];
    UIView *card = [UIView new];
    card.userInteractionEnabled = NO;
    card.backgroundColor = UIColor.secondarySystemBackgroundColor;
    card.layer.cornerRadius = 8;
    card.translatesAutoresizingMaskIntoConstraints = NO;
    UILabel *label = [UILabel new];
    label.text = text;
    label.font = [UIFont preferredFontForTextStyle:UIFontTextStyleSubheadline];
    label.adjustsFontForContentSizeCategory = YES;
    label.numberOfLines = 0;
    label.translatesAutoresizingMaskIntoConstraints = NO;
    [card addSubview:label];
    [window addSubview:card];
    [NSLayoutConstraint activateConstraints:@[
        [label.leadingAnchor constraintEqualToAnchor:card.leadingAnchor constant:12],
        [label.trailingAnchor constraintEqualToAnchor:card.trailingAnchor constant:-12],
        [label.topAnchor constraintEqualToAnchor:card.topAnchor constant:8],
        [label.bottomAnchor constraintEqualToAnchor:card.bottomAnchor constant:-8],
        [card.widthAnchor constraintLessThanOrEqualToAnchor:window.safeAreaLayoutGuide.widthAnchor constant:-24],
        [card.centerXAnchor constraintEqualToAnchor:window.safeAreaLayoutGuide.centerXAnchor],
        [card.bottomAnchor constraintEqualToAnchor:window.safeAreaLayoutGuide.bottomAnchor constant:-12]
    ]];
    objc_setAssociatedObject(window, &toastKey, card, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    __weak UIWindow *owner = window;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC), dispatch_get_main_queue(), ^{
        [card removeFromSuperview];
        if (owner && objc_getAssociatedObject(owner, &toastKey) == card)
            objc_setAssociatedObject(owner, &toastKey, nil, OBJC_ASSOCIATION_ASSIGN);
    });
}
