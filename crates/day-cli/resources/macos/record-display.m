// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The screen recorder for `day launch --script … --video` on macos-appkit. Embedded in day-cli
// (screenshot.rs), compiled once with the host's clang, and run as
//   day-record-display <display-id> <bundle-id> <fps> <out.mp4>
// while a scripted run drives the app on the capture display (capture-display.m).
//
// It records that display through ScreenCaptureKit, cropped to the app's main window's frame
// (title bar included, shadow not): the app as the person using it sees it, with its sheets,
// menus and popovers over the window, and none of the display's menu bar. Frames arrive when the picture changes and are written with
// their own times, HEVC at the display's pixel scale.
//
// Output on stdout:
//   started <unix-ms>   once the first frame is in the file: its time on the host's clock, the
//                       recording's time zero
//   finished            after stdin closes (the CLI's stop) and the file is complete
//   error <why>         (exit 1)
#import <AVFoundation/AVFoundation.h>
#import <Cocoa/Cocoa.h>
#import <CoreMedia/CoreMedia.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include <sys/time.h>

static void fail(NSString *why) {
    printf("error %s\n", why.UTF8String);
    fflush(stdout);
    exit(1);
}

// Unix milliseconds of a host-clock (mach) time, by its distance from now on both clocks.
static long long unix_ms_of(CMTime host) {
    struct timeval now;
    gettimeofday(&now, NULL);
    long long now_ms = (long long)now.tv_sec * 1000 + now.tv_usec / 1000;
    CMTime ago = CMTimeSubtract(CMClockGetTime(CMClockGetHostTimeClock()), host);
    return now_ms - (long long)(CMTimeGetSeconds(ago) * 1000.0);
}

@interface Recorder : NSObject <SCStreamOutput, SCStreamDelegate>
@property(strong) SCStream *stream;
@property(strong) AVAssetWriter *writer;
@property(strong) AVAssetWriterInput *input;
@property CMTime last;
@property BOOL started;
@end

@implementation Recorder
- (void)stream:(SCStream *)stream
    didOutputSampleBuffer:(CMSampleBufferRef)buffer
                   ofType:(SCStreamOutputType)type {
    if (type != SCStreamOutputTypeScreen || !CMSampleBufferIsValid(buffer)) return;
    // Only complete frames carry a picture; idle ones say nothing changed.
    CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(buffer, NO);
    if (!attachments || CFArrayGetCount(attachments) == 0) return;
    NSDictionary *info = (__bridge NSDictionary *)CFArrayGetValueAtIndex(attachments, 0);
    if ([info[SCStreamFrameInfoStatus] integerValue] != SCFrameStatusComplete) return;
    CMTime pts = CMSampleBufferGetPresentationTimeStamp(buffer);
    @synchronized(self) {
        if (!self.started) {
            [self.writer startSessionAtSourceTime:pts];
            self.started = YES;
            printf("started %lld\n", unix_ms_of(pts));
            fflush(stdout);
        }
        if (self.input.readyForMoreMediaData && [self.input appendSampleBuffer:buffer]) {
            self.last = pts;
        }
    }
}

- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error {
    fail([NSString stringWithFormat:@"the stream stopped: %@", error.localizedDescription]);
}

- (void)finish {
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    [self.stream stopCaptureWithCompletionHandler:^(NSError *error) {
      dispatch_semaphore_signal(done);
    }];
    dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC));
    @synchronized(self) {
        if (!self.started) fail(@"no frame was recorded");
        // The last picture holds until the stop, so a still ending is not cut short.
        [self.input markAsFinished];
        [self.writer endSessionAtSourceTime:CMClockGetTime(CMClockGetHostTimeClock())];
    }
    [self.writer finishWritingWithCompletionHandler:^{
      if (self.writer.status != AVAssetWriterStatusCompleted) {
          fail([NSString stringWithFormat:@"the file did not finish: %@",
                                          self.writer.error.localizedDescription]);
      }
      printf("finished\n");
      fflush(stdout);
      exit(0);
    }];
}
@end

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc < 5) {
            fprintf(stderr, "usage: day-record-display <display-id> <bundle-id> <fps> <out.mp4>\n");
            return 2;
        }
        CGDirectDisplayID display_id = (CGDirectDisplayID)strtoul(argv[1], NULL, 10);
        NSString *bundle = @(argv[2]);
        int fps = atoi(argv[3]);
        NSURL *out = [NSURL fileURLWithPath:@(argv[4])];

        // The app has its window up by the time the run's first step goes out, but the window
        // server can take a moment to list it: look for a while.
        __block SCDisplay *display = nil;
        __block SCRunningApplication *app = nil;
        __block SCWindow *window = nil;
        __block NSString *why = @"ScreenCaptureKit did not answer";
        for (int attempt = 0; attempt < 50 && !window; attempt++) {
            dispatch_semaphore_t answered = dispatch_semaphore_create(0);
            [SCShareableContent
                getShareableContentWithCompletionHandler:^(SCShareableContent *content,
                                                           NSError *error) {
                  if (error) {
                      why = error.localizedDescription;
                  }
                  for (SCDisplay *d in content.displays) {
                      if (d.displayID == display_id) display = d;
                  }
                  for (SCRunningApplication *a in content.applications) {
                      if ([a.bundleIdentifier isEqualToString:bundle]) app = a;
                  }
                  // The main window: the app's largest normal-level window on the display.
                  CGRect bounds = CGDisplayBounds(display_id);
                  CGFloat best = 0;
                  for (SCWindow *w in content.windows) {
                      if (w.owningApplication.processID != app.processID || !w.onScreen ||
                          w.windowLayer != 0 || !CGRectIntersectsRect(w.frame, bounds)) {
                          continue;
                      }
                      CGFloat area = w.frame.size.width * w.frame.size.height;
                      if (area > best) {
                          best = area;
                          window = w;
                      }
                  }
                  if (!error && !display) why = @"the capture display is not online";
                  if (!error && display && !app) why = @"the app is not running";
                  if (!error && app && !window) why = @"the app has no window on the display";
                  dispatch_semaphore_signal(answered);
                }];
            dispatch_semaphore_wait(answered, DISPATCH_TIME_FOREVER);
            if (!window) [NSThread sleepForTimeInterval:0.2];
        }
        if (!window) fail(why);

        // The window's frame on the display, in points, and the display's pixels per point.
        CGRect bounds = CGDisplayBounds(display_id);
        CGRect frame = CGRectOffset(window.frame, -bounds.origin.x, -bounds.origin.y);
        CGDisplayModeRef mode = CGDisplayCopyDisplayMode(display_id);
        double scale = mode && CGDisplayModeGetWidth(mode) > 0
                           ? (double)CGDisplayModeGetPixelWidth(mode) / CGDisplayModeGetWidth(mode)
                           : 1.0;
        if (mode) CGDisplayModeRelease(mode);
        size_t width = (size_t)(frame.size.width * scale) & ~(size_t)1;
        size_t height = (size_t)(frame.size.height * scale) & ~(size_t)1;

        // The whole display, cropped to the window below. A filter naming the app is window
        // sharing, and macOS then marks each shared window: its traffic lights give way to a
        // purple "being shared" badge in the recording. The display is the run's own, so the
        // app's windows are all there is on it.
        SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display
                                                          excludingWindows:@[]];
        SCStreamConfiguration *config = [SCStreamConfiguration new];
        config.sourceRect = frame;
        config.width = width;
        config.height = height;
        config.minimumFrameInterval = CMTimeMake(1, fps);
        config.showsCursor = NO;
        config.pixelFormat = kCVPixelFormatType_32BGRA;
        // sRGB pixels whatever the display's profile, encoded and tagged BT.709 below, so the
        // file decodes to the colors the app drew.
        config.colorSpaceName = kCGColorSpaceSRGB;
        config.queueDepth = 6;

        NSError *error = nil;
        [[NSFileManager defaultManager] removeItemAtURL:out error:nil];
        AVAssetWriter *writer = [AVAssetWriter assetWriterWithURL:out
                                                         fileType:AVFileTypeMPEG4
                                                            error:&error];
        if (!writer) fail(error.localizedDescription);
        // HEVC: a 2x desktop window is past what H.264's levels take at full size.
        AVAssetWriterInput *input = [AVAssetWriterInput
            assetWriterInputWithMediaType:AVMediaTypeVideo
                           outputSettings:@{
                               AVVideoCodecKey : AVVideoCodecTypeHEVC,
                               AVVideoWidthKey : @(width),
                               AVVideoHeightKey : @(height),
                               AVVideoColorPropertiesKey : @{
                                   AVVideoColorPrimariesKey : AVVideoColorPrimaries_ITU_R_709_2,
                                   AVVideoTransferFunctionKey : AVVideoTransferFunction_ITU_R_709_2,
                                   AVVideoYCbCrMatrixKey : AVVideoYCbCrMatrix_ITU_R_709_2,
                               },
                               AVVideoCompressionPropertiesKey : @{
                                   AVVideoAverageBitRateKey : @(24000000),
                                   AVVideoExpectedSourceFrameRateKey : @(fps),
                               },
                           }];
        input.expectsMediaDataInRealTime = YES;
        if (![writer canAddInput:input]) fail(@"the writer refused the video input");
        [writer addInput:input];
        if (![writer startWriting]) fail(writer.error.localizedDescription);

        Recorder *recorder = [Recorder new];
        recorder.writer = writer;
        recorder.input = input;
        recorder.stream = [[SCStream alloc] initWithFilter:filter
                                             configuration:config
                                                  delegate:recorder];
        dispatch_queue_t frames = dispatch_queue_create("day.record.frames", DISPATCH_QUEUE_SERIAL);
        if (![recorder.stream addStreamOutput:recorder
                                         type:SCStreamOutputTypeScreen
                           sampleHandlerQueue:frames
                                        error:&error]) {
            fail(error.localizedDescription);
        }
        [recorder.stream startCaptureWithCompletionHandler:^(NSError *error) {
          if (error) fail(error.localizedDescription);
        }];

        // Stopped by the CLI closing stdin, however it ends.
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
          char buffer[64];
          while (read(STDIN_FILENO, buffer, sizeof buffer) > 0) {
          }
          [recorder finish];
        });
        [[NSRunLoop currentRunLoop] run];
    }
    return 0;
}
