#import "AppDelegate.h"

#import <AppKit/AppKit.h>
#import <AVFoundation/AVFoundation.h>
#import <React/RCTBundleURLProvider.h>
#import <React/RCTBridgeModule.h>
#import <React/RCTComponent.h>
#import <React/RCTUIManager.h>
#import <React/RCTUIKit.h>
#import <React/RCTViewManager.h>
#if DEBUG
#import <React/RCTDevLoadingViewSetEnabled.h>
#endif

@interface AppDelegate () <NSWindowDelegate>
@property (nonatomic, strong) NSWindow *claroraMainWindow;
@end

@interface RNClipboard : NSObject <RCTBridgeModule>
@end

@implementation RNClipboard

RCT_EXPORT_MODULE(RNClipboard);

RCT_EXPORT_METHOD(setText:(NSString *)text)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    NSPasteboard *pasteboard = NSPasteboard.generalPasteboard;
    [pasteboard clearContents];
    [pasteboard setString:text ?: @"" forType:NSPasteboardTypeString];
  });
}

@end

@interface RNFilePicker : NSObject <RCTBridgeModule>
@end

@implementation RNFilePicker

RCT_EXPORT_MODULE(RNFilePicker);

RCT_EXPORT_METHOD(getDocumentDirectory:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSURL *directory = [[NSFileManager defaultManager] URLsForDirectory:NSApplicationSupportDirectory
                                                            inDomains:NSUserDomainMask].firstObject;
  NSURL *claroraDirectory = [directory URLByAppendingPathComponent:@"Clarora" isDirectory:YES];
  NSError *error = nil;
  [[NSFileManager defaultManager] createDirectoryAtURL:claroraDirectory
                           withIntermediateDirectories:YES
                                            attributes:nil
                                                 error:&error];
  if (error != nil) {
    reject(@"filesystem_error", error.localizedDescription, error);
    return;
  }
  resolve(claroraDirectory.path);
}

RCT_EXPORT_METHOD(pickFile:(NSDictionary *)options
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    panel.canChooseFiles = YES;
    panel.canChooseDirectories = NO;
    panel.allowsMultipleSelection = NO;
    panel.allowedFileTypes = [self fileTypesForOption:options[@"type"]];

    [panel beginWithCompletionHandler:^(NSModalResponse response) {
      if (response != NSModalResponseOK || panel.URL == nil) {
        resolve(nil);
        return;
      }

      NSURL *url = panel.URL;
      [panel orderOut:nil];
      dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        NSURL *resolvedURL = url;
        NSError *error = nil;
        BOOL accessed = [url startAccessingSecurityScopedResource];
        if ([options[@"copyToCacheDirectory"] boolValue]) {
          NSURL *directory = [[NSFileManager defaultManager] URLsForDirectory:NSApplicationSupportDirectory
                                                                     inDomains:NSUserDomainMask].firstObject;
          NSURL *importsDirectory = [[directory URLByAppendingPathComponent:@"Clarora" isDirectory:YES]
                                     URLByAppendingPathComponent:@"Imports" isDirectory:YES];
          [[NSFileManager defaultManager] createDirectoryAtURL:importsDirectory
                                    withIntermediateDirectories:YES
                                                     attributes:nil
                                                          error:&error];
          NSURL *destination = [importsDirectory URLByAppendingPathComponent:[NSString stringWithFormat:@"%@_%@", NSUUID.UUID.UUIDString, url.lastPathComponent]];
          if (error == nil) {
            [[NSFileManager defaultManager] copyItemAtURL:url toURL:destination error:&error];
          }
          resolvedURL = destination;
        }
        if (accessed) [url stopAccessingSecurityScopedResource];
        dispatch_async(dispatch_get_main_queue(), ^{
          if (error != nil) {
            reject(@"file_copy_error", error.localizedDescription, error);
            return;
          }
          resolve(@{
            @"uri": resolvedURL.path ?: @"",
            @"name": resolvedURL.lastPathComponent ?: @""
          });
        });
      });
    }];
  });
}

RCT_EXPORT_METHOD(pickDirectory:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  (void)reject;
  dispatch_async(dispatch_get_main_queue(), ^{
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    panel.canChooseFiles = NO;
    panel.canChooseDirectories = YES;
    panel.allowsMultipleSelection = NO;

    [panel beginWithCompletionHandler:^(NSModalResponse response) {
      if (response != NSModalResponseOK || panel.URL == nil) {
        resolve(nil);
        return;
      }
      NSURL *url = panel.URL;
      [panel orderOut:nil];
      resolve(url.path ?: @"");
    }];
  });
}

RCT_EXPORT_METHOD(listFiles:(NSString *)dirPath
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSFileManager *manager = [NSFileManager defaultManager];
  BOOL isDirectory = NO;
  if (![manager fileExistsAtPath:dirPath isDirectory:&isDirectory] || !isDirectory) {
    reject(@"filesystem_error", @"目录不存在", nil);
    return;
  }
  NSDirectoryEnumerator *enumerator = [manager enumeratorAtURL:[NSURL fileURLWithPath:dirPath]
                                    includingPropertiesForKeys:@[ NSURLIsRegularFileKey ]
                                                       options:NSDirectoryEnumerationSkipsPackageDescendants
                                                  errorHandler:nil];
  NSMutableArray<NSString *> *paths = [NSMutableArray array];
  for (NSURL *fileURL in enumerator) {
    if ([fileURL.lastPathComponent hasPrefix:@"."]) continue;
    NSNumber *isRegular = nil;
    [fileURL getResourceValue:&isRegular forKey:NSURLIsRegularFileKey error:nil];
    if (isRegular.boolValue) {
      [paths addObject:fileURL.path];
    }
  }
  resolve(paths);
}

RCT_EXPORT_METHOD(makeDirectory:(NSString *)path
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSError *error = nil;
  [[NSFileManager defaultManager] createDirectoryAtPath:path
                            withIntermediateDirectories:YES
                                             attributes:nil
                                                  error:&error];
  if (error != nil) {
    reject(@"filesystem_error", error.localizedDescription, error);
    return;
  }
  resolve(nil);
}

RCT_EXPORT_METHOD(copyFile:(NSString *)source
                  destination:(NSString *)destination
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSFileManager *manager = [NSFileManager defaultManager];
  NSError *error = nil;
  [manager createDirectoryAtPath:[destination stringByDeletingLastPathComponent]
     withIntermediateDirectories:YES
                      attributes:nil
                           error:&error];
  if (error == nil && [manager fileExistsAtPath:destination]) {
    [manager removeItemAtPath:destination error:&error];
  }
  if (error == nil) {
    [manager copyItemAtPath:source toPath:destination error:&error];
  }
  if (error != nil) {
    reject(@"filesystem_error", error.localizedDescription, error);
    return;
  }
  resolve(nil);
}

RCT_EXPORT_METHOD(readFile:(NSString *)path
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSError *error = nil;
  NSString *content = [NSString stringWithContentsOfFile:path encoding:NSUTF8StringEncoding error:&error];
  if (content == nil) {
    reject(@"filesystem_error", error.localizedDescription ?: @"无法读取文件", error);
    return;
  }
  resolve(content);
}

RCT_EXPORT_METHOD(readBase64:(NSString *)path
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSError *error = nil;
  NSData *data = [NSData dataWithContentsOfFile:path options:0 error:&error];
  if (!data) { reject(@"filesystem_error", error.localizedDescription, error); return; }
  resolve([data base64EncodedStringWithOptions:0]);
}

RCT_EXPORT_METHOD(writeFile:(NSString *)path
                  contents:(NSString *)contents
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSError *error = nil;
  [[NSFileManager defaultManager] createDirectoryAtPath:[path stringByDeletingLastPathComponent]
                            withIntermediateDirectories:YES
                                             attributes:nil
                                                  error:&error];
  BOOL wrote = NO;
  if (error == nil) {
    wrote = [contents writeToFile:path atomically:YES encoding:NSUTF8StringEncoding error:&error];
  }
  if (!wrote) {
    reject(@"filesystem_error", error.localizedDescription ?: @"写入文件失败", error);
    return;
  }
  resolve(nil);
}

RCT_EXPORT_METHOD(deleteFile:(NSString *)path
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSError *error = nil;
  if ([[NSFileManager defaultManager] fileExistsAtPath:path]) {
    [[NSFileManager defaultManager] removeItemAtPath:path error:&error];
  }
  if (error != nil) {
    reject(@"filesystem_error", error.localizedDescription, error);
    return;
  }
  resolve(nil);
}

- (NSArray<NSString *> *)fileTypesForOption:(NSString *)type
{
  if ([type isEqualToString:@"audio/*"]) {
    return @[@"mp3", @"m4a", @"wav", @"aac", @"aiff", @"flac", @"ogg"];
  }
  if ([type isEqualToString:@"text/*"]) {
    return @[@"txt", @"srt", @"vtt", @"md", @"csv"];
  }
  if ([type isEqualToString:@"video/*"]) {
    return @[@"mp4", @"mov", @"m4v", @"mkv", @"webm", @"avi"];
  }
  return nil;
}

@end

// The Fabric TextInput focus path on react-native-macos can crash while an
// AppKit context menu is handing control back to React.  Keep the question
// editor entirely in AppKit so selecting a subtitle can safely lead to a
// free-form question.
@interface RNQuestionPrompt : NSObject <RCTBridgeModule>
@end

@implementation RNQuestionPrompt

RCT_EXPORT_MODULE(RNQuestionPrompt);

+ (BOOL)requiresMainQueueSetup
{
  return YES;
}

RCT_EXPORT_METHOD(show:(NSString *)title
                  message:(NSString *)message
             defaultValue:(NSString *)defaultValue
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    NSWindow *window = NSApp.keyWindow ?: NSApp.mainWindow;
    if (window == nil) {
      reject(@"question_prompt_unavailable", @"无法显示提问窗口", nil);
      return;
    }

    NSAlert *alert = [[NSAlert alloc] init];
    alert.messageText = title.length > 0 ? title : @"向 AI 提问";
    alert.informativeText = message ?: @"";
    [alert addButtonWithTitle:@"提问"];
    [alert addButtonWithTitle:@"取消"];

    NSTextField *input = [[NSTextField alloc] initWithFrame:NSMakeRect(0, 0, 420, 24)];
    input.stringValue = defaultValue ?: @"";
    input.placeholderString = @"输入你的问题";
    alert.accessoryView = input;

    [alert beginSheetModalForWindow:window completionHandler:^(NSModalResponse response) {
      if (response == NSAlertFirstButtonReturn) {
        resolve(input.stringValue ?: @"");
      } else {
        resolve([NSNull null]);
      }
    }];
  });
}

@end

@interface RNMacAudio : NSObject <RCTBridgeModule, AVAudioPlayerDelegate>
@property(nonatomic, strong) AVAudioPlayer *player;
@property(nonatomic, strong) NSMutableDictionary<NSNumber *, AVAudioPlayer *> *feedPlayers;
// AVAudioPlayer resets currentTime to 0 once playback finishes, so comparing
// currentTime against duration can never detect completion; the delegate
// callback is the only reliable signal, mirrored from ClaroraAudioModule.kt.
@property(nonatomic, assign) BOOL playbackCompleted;
@end

@implementation RNMacAudio

RCT_EXPORT_MODULE(RNMacAudio);

// Independent two-slot players: preparing the next discovery item must never
// replace the listening/flashcard player or interrupt the current feed item.
RCT_EXPORT_METHOD(feedPrepare:(nonnull NSNumber *)slot path:(NSString *)path start:(nonnull NSNumber *)start resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  if (!self.feedPlayers) self.feedPlayers = [NSMutableDictionary new];
  [self.feedPlayers[slot] stop];
  [self.feedPlayers removeObjectForKey:slot];
  NSError *error = nil;
  AVAudioPlayer *p = [[AVAudioPlayer alloc] initWithContentsOfURL:[NSURL fileURLWithPath:path] error:&error];
  if (!p) { reject(@"feed_audio", error.localizedDescription ?: @"无法加载音频", error); return; }
  p.currentTime = MIN(start.doubleValue / 1000.0, p.duration);
  [p prepareToPlay];
  self.feedPlayers[slot] = p;
  resolve(nil);
}

RCT_EXPORT_METHOD(feedPlay:(nonnull NSNumber *)slot resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  for (NSNumber *key in self.feedPlayers) if (![key isEqual:slot]) [self.feedPlayers[key] pause];
  AVAudioPlayer *p = self.feedPlayers[slot];
  if (!p || ![p play]) { reject(@"feed_audio", @"音频尚未就绪", nil); return; }
  resolve(nil);
}

RCT_EXPORT_METHOD(feedPause:(nonnull NSNumber *)slot resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  [self.feedPlayers[slot] pause];
  resolve(nil);
}

RCT_EXPORT_METHOD(feedStatus:(nonnull NSNumber *)slot resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  AVAudioPlayer *p = self.feedPlayers[slot];
  resolve(@{@"positionMillis": @(p.currentTime * 1000), @"isPlaying": @(p.isPlaying)});
}

RCT_EXPORT_METHOD(feedUnload:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  for (AVAudioPlayer *p in self.feedPlayers.allValues) [p stop];
  [self.feedPlayers removeAllObjects];
  resolve(nil);
}

RCT_EXPORT_METHOD(load:(NSString *)path
                  rate:(nonnull NSNumber *)rate
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSError *error = nil;
  self.player = [[AVAudioPlayer alloc] initWithContentsOfURL:[NSURL fileURLWithPath:path] error:&error];
  if (self.player == nil) {
    reject(@"audio_load_error", error.localizedDescription ?: @"无法加载音频", error);
    return;
  }
  self.player.delegate = self;
  self.playbackCompleted = NO;
  self.player.enableRate = YES;
  self.player.rate = rate.floatValue;
  [self.player prepareToPlay];
  resolve([self status]);
}

RCT_EXPORT_METHOD(play:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  if (self.player == nil || ![self.player play]) {
    reject(@"audio_play_error", @"没有可播放的音频", nil);
    return;
  }
  self.playbackCompleted = NO;
  resolve([self status]);
}

RCT_EXPORT_METHOD(pause:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  [self.player pause];
  resolve([self status]);
}

RCT_EXPORT_METHOD(setPosition:(nonnull NSNumber *)milliseconds
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  if (self.player == nil) {
    reject(@"audio_seek_error", @"没有已加载的音频", nil);
    return;
  }
  self.player.currentTime = MAX(0, MIN(milliseconds.doubleValue / 1000.0, self.player.duration));
  self.playbackCompleted = NO;
  resolve([self status]);
}

RCT_EXPORT_METHOD(setRate:(nonnull NSNumber *)rate
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  if (self.player == nil) {
    resolve(nil);
    return;
  }
  self.player.enableRate = YES;
  self.player.rate = rate.floatValue;
  resolve([self status]);
}

RCT_EXPORT_METHOD(setLoop:(BOOL)loop
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  if (self.player != nil) {
    self.player.numberOfLoops = loop ? -1 : 0;
  }
  resolve([self status]);
}

RCT_EXPORT_METHOD(status:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  resolve([self status]);
}

RCT_EXPORT_METHOD(unload:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  [self.player stop];
  self.player = nil;
  self.playbackCompleted = NO;
  resolve(nil);
}

- (void)audioPlayerDidFinishPlaying:(AVAudioPlayer *)player successfully:(BOOL)flag
{
  self.playbackCompleted = YES;
}

// Concatenate audio files into one M4A. AVFoundation re-encodes on export, so
// mixed containers / sample rates / channel counts are fine (short segments
// land on separate composition tracks automatically). Resolves with each
// segment's duration in ms so the JS side can shift subtitles while merging.
RCT_EXPORT_METHOD(mergeAudios:(NSArray<NSString *> *)paths
                  outputPath:(NSString *)outputPath
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSMutableArray<AVURLAsset *> *assets = [NSMutableArray array];
  for (NSString *path in paths) {
    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:[NSURL fileURLWithPath:path] options:nil];
    if ([[asset tracksWithMediaType:AVMediaTypeAudio] firstObject] == nil) {
      reject(@"audio_merge_error", [NSString stringWithFormat:@"无法读取音频：%@", path], nil);
      return;
    }
    [assets addObject:asset];
  }

  AVMutableComposition *composition = [AVMutableComposition composition];
  NSMutableArray<NSNumber *> *segmentMs = [NSMutableArray array];
  CMTime cursor = kCMTimeZero;
  for (AVURLAsset *asset in assets) {
    CMTimeRange range = CMTimeRangeMake(kCMTimeZero, asset.duration);
    NSError *insertError = nil;
    if (![composition insertTimeRange:range ofAsset:asset atTime:cursor error:&insertError]) {
      reject(@"audio_merge_error", insertError.localizedDescription ?: @"音频拼接失败", insertError);
      return;
    }
    cursor = CMTimeAdd(cursor, asset.duration);
    [segmentMs addObject:@(CMTimeGetSeconds(asset.duration) * 1000.0)];
  }

  AVAssetExportSession *session = [AVAssetExportSession exportSessionWithAsset:composition presetName:AVAssetExportPresetAppleM4A];
  if (session == nil) {
    reject(@"audio_merge_error", @"无法创建音频导出会话", nil);
    return;
  }
  [[NSFileManager defaultManager] removeItemAtPath:outputPath error:nil];
  session.outputURL = [NSURL fileURLWithPath:outputPath];
  session.outputFileType = AVFileTypeAppleM4A;
  [session exportAsynchronouslyWithCompletionHandler:^{
    if (session.status == AVAssetExportSessionStatusCompleted) {
      resolve(@{
        @"durationMs": @(CMTimeGetSeconds(cursor) * 1000.0),
        @"segmentDurationMs": segmentMs,
      });
    } else {
      reject(@"audio_merge_error", session.error.localizedDescription ?: @"音频合并导出失败", session.error);
    }
  }];
}

- (NSDictionary *)status
{
  if (self.player == nil) {
    return @{ @"isLoaded": @NO, @"positionMillis": @0, @"isPlaying": @NO };
  }
  BOOL finished = self.playbackCompleted && !self.player.isPlaying;
  return @{
    @"isLoaded": @YES,
    @"positionMillis": @(self.player.currentTime * 1000),
    @"durationMillis": @(self.player.duration * 1000),
    @"isPlaying": @(self.player.isPlaying),
    @"didJustFinish": @(finished)
  };
}

@end

// Fabric's macOS Paragraph view paints text but does not back `selectable`
// with an NSTextView. The listening transcript needs a real AppKit text view
// so a mouse drag creates an actual selection range.
@class RNSelectableSubtitleView;

@interface RNSelectableSubtitleTextView : NSTextView
@property (nonatomic, weak) RNSelectableSubtitleView *subtitleOwner;
@property (nonatomic, strong) id tapMonitor;
@property (nonatomic, weak) NSWindow *tapWindow;
@property (nonatomic, assign) NSPoint tapStartLocation;
@property (nonatomic, assign) NSUInteger tapCharacterIndex;
@property (nonatomic, assign) BOOL tapDidDrag;
// NSAttributedString background colors are rectangular and cannot
// reproduce the compact rounded active-word chip used by the old RN layout.
// Keep the range separately so we can draw that chip after AppKit has drawn
// normal cue backgrounds, then redraw only the word's glyphs above it.
@property (nonatomic, assign) NSRange activeWordRange;
@property (nonatomic, strong) NSColor *activeWordPillColor;
@property (nonatomic, assign) NSInteger loopStartMarkerIndex;
@property (nonatomic, assign) BOOL loopStartMarkerAfter;
@property (nonatomic, assign) NSInteger loopEndMarkerIndex;
@property (nonatomic, assign) BOOL loopEndMarkerAfter;
@property (nonatomic, assign) BOOL cardMode;
- (NSUInteger)characterIndexForEvent:(NSEvent *)event;
- (void)stopTapTracking;
@end

@interface RNSelectableSubtitleView : RCTUIView
@property (nonatomic, copy) NSString *text;
@property (nonatomic, assign) CGFloat fontSize;
@property (nonatomic, assign) BOOL selectionEnabled;
@property (nonatomic, strong) NSColor *textColor;
// All offsets use UTF-16 indices, matching JavaScript's String.length and
// String#indexOf values.  A negative start or a zero length disables that
// particular highlight.
@property (nonatomic, assign) NSInteger activeCueStart;
@property (nonatomic, assign) NSInteger activeCueLength;
@property (nonatomic, assign) NSInteger activeWordStart;
@property (nonatomic, assign) NSInteger activeWordLength;
@property (nonatomic, strong) NSColor *activeCueTextColor;
@property (nonatomic, strong) NSColor *activeAccentColor;
@property (nonatomic, strong) NSColor *activeWordTextColor;
// AI 标记的重点区间：JSON 数组的 [start, length] 对（UTF-16 索引，与
// activeCue* 属性同一坐标系），以下划线标出，不参与播放高亮。
@property (nonatomic, copy) NSString *keyRangesJson;
@property (nonatomic, strong) NSColor *keyHighlightColor;
@property (nonatomic, assign) NSInteger loopStartMarkerIndex;
@property (nonatomic, assign) BOOL loopStartMarkerAfter;
@property (nonatomic, assign) NSInteger loopEndMarkerIndex;
@property (nonatomic, assign) BOOL loopEndMarkerAfter;
@property (nonatomic, copy) RCTDirectEventBlock onAskSelection;
@property (nonatomic, copy) RCTDirectEventBlock onTapAtCharacter;
@property (nonatomic, copy) RCTDirectEventBlock onScrollPositionChange;
@property (nonatomic, assign) BOOL cardMode;
@property (nonatomic, assign) CGFloat contentOffsetY;
- (void)askSelectionFromTextView:(NSTextView *)textView;
- (void)tapAtCharacterIndex:(NSUInteger)index;
- (void)scrollActiveCueIntoView;
@end

@implementation RNSelectableSubtitleTextView

- (instancetype)initWithFrame:(NSRect)frameRect
{
  if (self = [super initWithFrame:frameRect]) {
    _activeWordRange = NSMakeRange(NSNotFound, 0);
    _loopStartMarkerIndex = -1;
    _loopEndMarkerIndex = -1;
  }
  return self;
}

- (void)dealloc
{
  [self stopTapTracking];
}

- (void)resetCursorRects
{
  // The transcript is a study surface rather than an editable text field.
  // Keep its familiar arrow cursor while leaving NSTextView selection intact.
  [self addCursorRect:self.bounds cursor:NSCursor.arrowCursor];
}

- (void)cursorUpdate:(NSEvent *)event
{
  // NSTextView reassigns an I-beam during cursor updates even when it is
  // non-editable, so override that last step as well.
  [NSCursor.arrowCursor set];
}

- (void)mouseMoved:(NSEvent *)event
{
  [super mouseMoved:event];
  [NSCursor.arrowCursor set];
}

- (void)mouseDown:(NSEvent *)event
{
  NSEventModifierFlags modifiers =
      event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
  const NSEventModifierFlags selectionModifiers =
      NSEventModifierFlagShift |
      NSEventModifierFlagControl |
      NSEventModifierFlagOption |
      NSEventModifierFlagCommand;
  const BOOL isPlainSingleLeftClick =
      event.type == NSEventTypeLeftMouseDown &&
      event.clickCount == 1 &&
      (modifiers & selectionModifiers) == 0;
  NSUInteger index = isPlainSingleLeftClick ? [self characterIndexForEvent:event] : NSNotFound;
  if (index == NSNotFound) {
    [super mouseDown:event];
    return;
  }

  [self stopTapTracking];
  self.tapWindow = event.window;
  self.tapStartLocation = event.locationInWindow;
  self.tapCharacterIndex = index;
  self.tapDidDrag = NO;

  __weak RNSelectableSubtitleTextView *weakSelf = self;
  self.tapMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:(NSEventMaskLeftMouseDragged | NSEventMaskLeftMouseUp)
                                                           handler:^NSEvent *(NSEvent *trackedEvent) {
    RNSelectableSubtitleTextView *strongSelf = weakSelf;
    if (strongSelf == nil) return trackedEvent;

    BOOL isSameWindow = trackedEvent.window == strongSelf.tapWindow;
    if (trackedEvent.type == NSEventTypeLeftMouseDragged) {
      if (isSameWindow) strongSelf.tapDidDrag = YES;
    }
    return trackedEvent;
  }];

  // NSTextView owns its normal click and drag-selection tracking.  The local
  // monitor above only observes its events and never consumes them.
  [super mouseDown:event];

  // `mouseDown:` returns after NSTextView has completed its selection
  // tracking.  A non-empty selected range is the reliable fallback on AppKit
  // paths that process drag events inside NSTextView's tracking loop instead
  // of dispatching them through a local event monitor.
  NSEvent *lastEvent = NSApp.currentEvent;
  CGFloat dx = lastEvent.locationInWindow.x - self.tapStartLocation.x;
  CGFloat dy = lastEvent.locationInWindow.y - self.tapStartLocation.y;
  BOOL pointerMoved =
      lastEvent.type == NSEventTypeLeftMouseUp &&
      lastEvent.window == self.tapWindow &&
      (dx * dx + dy * dy) > 4.0;
  NSRange selectedRange = self.selectedRange;
  BOOL madeTextSelection =
      selectedRange.location != NSNotFound && selectedRange.length > 0;
  NSUInteger tappedIndex = self.tapCharacterIndex;
  BOOL shouldEmit = !self.tapDidDrag && !pointerMoved && !madeTextSelection;
  [self stopTapTracking];
  if (shouldEmit) {
    [self.subtitleOwner tapAtCharacterIndex:tappedIndex];
  }
}

- (NSUInteger)characterIndexForEvent:(NSEvent *)event
{
  NSUInteger characterCount = self.string.length;
  NSLayoutManager *layoutManager = self.layoutManager;
  NSTextContainer *textContainer = self.textContainer;
  if (characterCount == 0 || layoutManager == nil || textContainer == nil) return NSNotFound;

  NSPoint point = [self convertPoint:event.locationInWindow fromView:nil];
  NSPoint containerPoint = NSMakePoint(
      point.x - self.textContainerOrigin.x,
      point.y - self.textContainerOrigin.y);
  if (!NSPointInRect(containerPoint, [layoutManager usedRectForTextContainer:textContainer])) {
    return NSNotFound;
  }

  NSUInteger glyphCount = layoutManager.numberOfGlyphs;
  if (glyphCount == 0) return NSNotFound;
  NSUInteger glyphIndex = [layoutManager glyphIndexForPoint:containerPoint
                                             inTextContainer:textContainer
                              fractionOfDistanceThroughGlyph:NULL];
  if (glyphIndex == NSNotFound || glyphIndex >= glyphCount) return NSNotFound;

  NSUInteger characterIndex = [layoutManager characterIndexForGlyphAtIndex:glyphIndex];
  return characterIndex < characterCount ? characterIndex : characterCount - 1;
}

- (void)stopTapTracking
{
  if (self.tapMonitor != nil) {
    [NSEvent removeMonitor:self.tapMonitor];
    self.tapMonitor = nil;
  }
  self.tapWindow = nil;
  self.tapDidDrag = NO;
}

- (void)setActiveWordRange:(NSRange)activeWordRange
{
  if (NSEqualRanges(_activeWordRange, activeWordRange)) return;
  _activeWordRange = activeWordRange;
  [self setNeedsDisplay:YES];
}

- (void)setActiveWordPillColor:(NSColor *)activeWordPillColor
{
  if ([_activeWordPillColor isEqual:activeWordPillColor]) return;
  _activeWordPillColor = activeWordPillColor;
  [self setNeedsDisplay:YES];
}

- (BOOL)hasTextSelectionIntersectingRange:(NSRange)range
{
  NSRange selection = self.selectedRange;
  if (selection.location == NSNotFound) return NO;
  if (selection.length == 0) {
    return NSLocationInRange(selection.location, range);
  }
  return NSIntersectionRange(selection, range).length > 0;
}

- (void)drawActiveWordPillInDirtyRect:(NSRect)dirtyRect
{
  NSRange characterRange = self.activeWordRange;
  if (characterRange.location == NSNotFound || characterRange.length == 0 ||
      NSMaxRange(characterRange) > self.string.length ||
      self.activeWordPillColor == nil ||
      [self hasTextSelectionIntersectingRange:characterRange]) {
    return;
  }

  NSLayoutManager *layoutManager = self.layoutManager;
  NSTextContainer *textContainer = self.textContainer;
  if (layoutManager == nil || textContainer == nil) return;
  [layoutManager ensureLayoutForTextContainer:textContainer];
  NSRange glyphRange = [layoutManager glyphRangeForCharacterRange:characterRange
                                               actualCharacterRange:NULL];
  if (glyphRange.length == 0) return;

  NSPoint origin = self.textContainerOrigin;
  [NSGraphicsContext saveGraphicsState];
  [self.activeWordPillColor setFill];
  [layoutManager enumerateEnclosingRectsForGlyphRange:glyphRange
                               withinSelectedGlyphRange:NSMakeRange(NSNotFound, 0)
                                        inTextContainer:textContainer
                                             usingBlock:^(NSRect glyphRect, BOOL *stop) {
    NSRect pillRect = NSOffsetRect(glyphRect, origin.x, origin.y);
    // Match the old word style: 3pt horizontal and 1pt vertical padding,
    // with a 4pt rounded corner. Each wrapped line gets its own pill.
    pillRect = NSInsetRect(pillRect, -3.0, -1.0);
    if (!NSIntersectsRect(pillRect, dirtyRect)) return;
    CGFloat radius = MIN(4.0, MIN(NSWidth(pillRect), NSHeight(pillRect)) / 2.0);
    [[NSBezierPath bezierPathWithRoundedRect:pillRect xRadius:radius yRadius:radius] fill];
  }];
  [NSGraphicsContext restoreGraphicsState];

  // The normal NSTexView pass has already drawn the glyphs.  Draw the active
  // word again after its custom background, so its white foreground remains
  // crisp while the rest of the transcript and AppKit selection stay native.
  [layoutManager drawGlyphsForGlyphRange:glyphRange atPoint:origin];
}

- (void)drawLoopMarkerAtCharacterIndex:(NSInteger)markerIndex
                         afterCharacter:(BOOL)afterCharacter
                            pointsRight:(BOOL)pointsRight
                            inDirtyRect:(NSRect)dirtyRect
{
  NSUInteger characterCount = self.string.length;
  if (markerIndex < 0 || characterCount == 0 || (NSUInteger)markerIndex >= characterCount ||
      self.activeWordPillColor == nil) {
    return;
  }
  NSRange selectionRange = NSMakeRange((NSUInteger)markerIndex, 1);
  if ([self hasTextSelectionIntersectingRange:selectionRange]) return;

  NSLayoutManager *layoutManager = self.layoutManager;
  NSTextContainer *textContainer = self.textContainer;
  if (layoutManager == nil || textContainer == nil) return;
  [layoutManager ensureLayoutForTextContainer:textContainer];
  NSRange glyphRange = [layoutManager glyphRangeForCharacterRange:selectionRange
                                               actualCharacterRange:NULL];
  if (glyphRange.length == 0) return;

  NSRect glyphRect = [layoutManager boundingRectForGlyphRange:glyphRange
                                               inTextContainer:textContainer];
  NSRect lineRect = [layoutManager lineFragmentRectForGlyphAtIndex:glyphRange.location
                                                      effectiveRange:NULL];
  NSPoint origin = self.textContainerOrigin;
  CGFloat x = (afterCharacter ? NSMaxX(glyphRect) : NSMinX(glyphRect)) + origin.x;
  CGFloat top = NSMinY(lineRect) + origin.y + 1.0;
  CGFloat bottom = NSMaxY(lineRect) + origin.y - 1.0;
  CGFloat middle = (top + bottom) / 2.0;
  NSRect markerRect = NSMakeRect(x - 13.0, top, 26.0, MAX(1.0, bottom - top));
  if (!NSIntersectsRect(markerRect, dirtyRect)) return;

  [NSGraphicsContext saveGraphicsState];
  [self.activeWordPillColor setStroke];
  NSBezierPath *line = [NSBezierPath bezierPath];
  line.lineWidth = 2.0;
  [line moveToPoint:NSMakePoint(x, top)];
  [line lineToPoint:NSMakePoint(x, bottom)];
  [line stroke];

  // The arrow is always on the inside of the captured range: A points right
  // from its line, while B points left toward its line.
  [self.activeWordPillColor setFill];
  NSBezierPath *arrow = [NSBezierPath bezierPath];
  if (pointsRight) {
    [arrow moveToPoint:NSMakePoint(x + 3.0, middle - 6.0)];
    [arrow lineToPoint:NSMakePoint(x + 3.0, middle + 6.0)];
    [arrow lineToPoint:NSMakePoint(x + 12.0, middle)];
  } else {
    [arrow moveToPoint:NSMakePoint(x - 3.0, middle - 6.0)];
    [arrow lineToPoint:NSMakePoint(x - 3.0, middle + 6.0)];
    [arrow lineToPoint:NSMakePoint(x - 12.0, middle)];
  }
  [arrow closePath];
  [arrow fill];
  [NSGraphicsContext restoreGraphicsState];
}

- (void)drawLoopMarkersInDirtyRect:(NSRect)dirtyRect
{
  [self drawLoopMarkerAtCharacterIndex:self.loopStartMarkerIndex
                        afterCharacter:self.loopStartMarkerAfter
                           pointsRight:YES
                           inDirtyRect:dirtyRect];
  [self drawLoopMarkerAtCharacterIndex:self.loopEndMarkerIndex
                        afterCharacter:self.loopEndMarkerAfter
                           pointsRight:NO
                           inDirtyRect:dirtyRect];
}

- (void)drawRect:(NSRect)dirtyRect
{
  [super drawRect:dirtyRect];
  [self drawActiveWordPillInDirtyRect:dirtyRect];
  [self drawLoopMarkersInDirtyRect:dirtyRect];
}

- (NSMenu *)menuForEvent:(NSEvent *)event
{
  if (!self.selectable) return nil;
  if (self.cardMode) return [super menuForEvent:event];
  NSRange range = self.selectedRange;
  if (range.location == NSNotFound || range.length == 0) {
    return [super menuForEvent:event];
  }

  NSMenu *menu = [[NSMenu alloc] initWithTitle:@"字幕操作"];
  NSMenuItem *copyItem = [[NSMenuItem alloc] initWithTitle:@"复制字幕"
                                                     action:@selector(copy:)
                                              keyEquivalent:@""];
  copyItem.target = self;
  [menu addItem:copyItem];

  NSMenuItem *askItem = [[NSMenuItem alloc] initWithTitle:@"向 AI 提问"
                                                    action:@selector(askSelection:)
                                             keyEquivalent:@""];
  askItem.target = self;
  [menu addItem:askItem];
  return menu;
}

- (void)askSelection:(id)sender
{
  [self.subtitleOwner askSelectionFromTextView:self];
}

@end

@implementation RNSelectableSubtitleView {
  NSScrollView *_scrollView;
  RNSelectableSubtitleTextView *_textView;
  NSArray<NSValue *> *_keyRanges;
}

- (instancetype)initWithFrame:(NSRect)frame
{
  if (self = [super initWithFrame:frame]) {
    _fontSize = 14;
    _selectionEnabled = YES;
    _textColor = NSColor.labelColor;
    _activeCueStart = -1;
    _activeCueLength = 0;
    _activeWordStart = -1;
    _activeWordLength = 0;
    _loopStartMarkerIndex = -1;
    _loopEndMarkerIndex = -1;
    _activeCueTextColor = NSColor.labelColor;
    _activeAccentColor = NSColor.controlAccentColor;
    _activeWordTextColor = NSColor.whiteColor;

    _scrollView = [[NSScrollView alloc] initWithFrame:self.bounds];
    _scrollView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    _scrollView.hasVerticalScroller = YES;
    _scrollView.hasHorizontalScroller = NO;
    _scrollView.autohidesScrollers = YES;
    _scrollView.drawsBackground = NO;
    _scrollView.contentView.postsBoundsChangedNotifications = YES;
    [[NSNotificationCenter defaultCenter] addObserver:self
                                             selector:@selector(scrollBoundsDidChange:)
                                                 name:NSViewBoundsDidChangeNotification
                                               object:_scrollView.contentView];

    _textView = [[RNSelectableSubtitleTextView alloc] initWithFrame:self.bounds];
    _textView.subtitleOwner = self;
    _textView.editable = NO;
    _textView.selectable = _selectionEnabled;
    _textView.drawsBackground = NO;
    _textView.richText = NO;
    _textView.usesFindBar = NO;
    _textView.focusRingType = NSFocusRingTypeNone;
    _textView.textContainerInset = NSMakeSize(14, 12);
    _textView.minSize = NSMakeSize(0, 0);
    _textView.maxSize = NSMakeSize(CGFLOAT_MAX, CGFLOAT_MAX);
    _textView.verticallyResizable = YES;
    _textView.horizontallyResizable = NO;
    _textView.autoresizingMask = NSViewWidthSizable;
    _textView.textContainer.containerSize = NSMakeSize(0, CGFLOAT_MAX);
    _textView.textContainer.widthTracksTextView = YES;
    _scrollView.documentView = _textView;
    [self addSubview:_scrollView];
    [self refreshText];
  }
  return self;
}

- (void)dealloc
{
  [[NSNotificationCenter defaultCenter] removeObserver:self];
}

- (void)setCardMode:(BOOL)cardMode
{
  if (_cardMode == cardMode) return;
  _cardMode = cardMode;
  _textView.cardMode = cardMode;
  _scrollView.hasVerticalScroller = !cardMode;
  [self refreshText];
}

- (void)setContentOffsetY:(CGFloat)contentOffsetY
{
  _contentOffsetY = MAX(0, contentOffsetY);
  NSClipView *clipView = _scrollView.contentView;
  NSView *documentView = _scrollView.documentView;
  CGFloat maxOffset = MAX(0, NSHeight(documentView.bounds) - NSHeight(clipView.bounds));
  CGFloat offset = MIN(_contentOffsetY, maxOffset);
  [clipView scrollToPoint:NSMakePoint(NSMinX(clipView.bounds), offset)];
  [_scrollView reflectScrolledClipView:clipView];
}

- (void)scrollBoundsDidChange:(NSNotification *)notification
{
  if (!_cardMode || self.onScrollPositionChange == nil) return;
  NSClipView *clipView = _scrollView.contentView;
  NSView *documentView = _scrollView.documentView;
  CGFloat contentHeight = NSHeight(documentView.bounds);
  CGFloat viewportHeight = NSHeight(clipView.bounds);
  CGFloat maxOffset = MAX(0, contentHeight - viewportHeight);
  CGFloat offset = MIN(MAX(0, NSMinY(clipView.bounds)), maxOffset);
  _contentOffsetY = offset;
  self.onScrollPositionChange(@{
    @"offset": @(offset),
    @"content": @(contentHeight),
    @"viewport": @(viewportHeight),
  });
}

- (void)setText:(NSString *)text
{
  NSString *next = text ?: @"";
  if ([_text isEqualToString:next]) return;
  _text = [next copy];
  [self refreshText];
}

- (void)setFontSize:(CGFloat)fontSize
{
  CGFloat next = fontSize > 0 ? fontSize : 14;
  if (_fontSize == next) return;
  _fontSize = next;
  [self refreshText];
}

- (void)setSelectionEnabled:(BOOL)selectionEnabled
{
  if (_selectionEnabled == selectionEnabled) return;
  _selectionEnabled = selectionEnabled;
  _textView.selectable = selectionEnabled;
  if (!selectionEnabled) {
    _textView.selectedRange = NSMakeRange(0, 0);
  }
}

- (void)setTextColor:(NSColor *)textColor
{
  NSColor *next = textColor ?: NSColor.labelColor;
  if ([_textColor isEqual:next]) return;
  _textColor = next;
  [self refreshText];
}

- (void)setActiveCueStart:(NSInteger)activeCueStart
{
  if (_activeCueStart == activeCueStart) return;
  _activeCueStart = activeCueStart;
  [self refreshText];
  [self scrollActiveCueIntoView];
}

- (void)setActiveCueLength:(NSInteger)activeCueLength
{
  if (_activeCueLength == activeCueLength) return;
  _activeCueLength = activeCueLength;
  [self refreshText];
  [self scrollActiveCueIntoView];
}

- (void)setActiveWordStart:(NSInteger)activeWordStart
{
  if (_activeWordStart == activeWordStart) return;
  _activeWordStart = activeWordStart;
  [self refreshText];
}

- (void)setActiveWordLength:(NSInteger)activeWordLength
{
  if (_activeWordLength == activeWordLength) return;
  _activeWordLength = activeWordLength;
  [self refreshText];
}

- (void)setActiveCueTextColor:(NSColor *)activeCueTextColor
{
  NSColor *next = activeCueTextColor ?: NSColor.labelColor;
  if ([_activeCueTextColor isEqual:next]) return;
  _activeCueTextColor = next;
  [self refreshText];
}

- (void)setActiveAccentColor:(NSColor *)activeAccentColor
{
  NSColor *next = activeAccentColor ?: NSColor.controlAccentColor;
  if ([_activeAccentColor isEqual:next]) return;
  _activeAccentColor = next;
  [self refreshText];
}

- (void)setActiveWordTextColor:(NSColor *)activeWordTextColor
{
  NSColor *next = activeWordTextColor ?: NSColor.whiteColor;
  if ([_activeWordTextColor isEqual:next]) return;
  _activeWordTextColor = next;
  [self refreshText];
}

- (void)setLoopStartMarkerIndex:(NSInteger)loopStartMarkerIndex
{
  if (_loopStartMarkerIndex == loopStartMarkerIndex) return;
  _loopStartMarkerIndex = loopStartMarkerIndex;
  [self refreshText];
}

- (void)setLoopStartMarkerAfter:(BOOL)loopStartMarkerAfter
{
  if (_loopStartMarkerAfter == loopStartMarkerAfter) return;
  _loopStartMarkerAfter = loopStartMarkerAfter;
  [self refreshText];
}

- (void)setLoopEndMarkerIndex:(NSInteger)loopEndMarkerIndex
{
  if (_loopEndMarkerIndex == loopEndMarkerIndex) return;
  _loopEndMarkerIndex = loopEndMarkerIndex;
  [self refreshText];
}

- (void)setLoopEndMarkerAfter:(BOOL)loopEndMarkerAfter
{
  if (_loopEndMarkerAfter == loopEndMarkerAfter) return;
  _loopEndMarkerAfter = loopEndMarkerAfter;
  [self refreshText];
}

- (void)setKeyRangesJson:(NSString *)keyRangesJson
{
  NSString *next = keyRangesJson ?: @"[]";
  if ([_keyRangesJson isEqualToString:next]) return;
  _keyRangesJson = [next copy];
  NSMutableArray<NSValue *> *ranges = [NSMutableArray array];
  NSData *data = [_keyRangesJson dataUsingEncoding:NSUTF8StringEncoding];
  if (data != nil) {
    id parsed = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    if ([parsed isKindOfClass:[NSArray class]]) {
      for (id item in parsed) {
        if (![item isKindOfClass:[NSArray class]] || [item count] < 2) continue;
        NSInteger start = [[item objectAtIndex:0] integerValue];
        NSInteger length = [[item objectAtIndex:1] integerValue];
        if (start < 0 || length <= 0) continue;
        [ranges addObject:[NSValue valueWithRange:NSMakeRange((NSUInteger)start, (NSUInteger)length)]];
      }
    }
  }
  _keyRanges = ranges;
  [self refreshText];
}

- (void)setKeyHighlightColor:(NSColor *)keyHighlightColor
{
  NSColor *next = keyHighlightColor ?: NSColor.systemOrangeColor;
  if ([_keyHighlightColor isEqual:next]) return;
  _keyHighlightColor = next;
  [self refreshText];
}

- (NSRange)highlightRangeAtStart:(NSInteger)start length:(NSInteger)length textLength:(NSUInteger)textLength
{
  if (start < 0 || length <= 0 || textLength == 0 || (NSUInteger)start >= textLength) {
    return NSMakeRange(NSNotFound, 0);
  }
  NSUInteger location = (NSUInteger)start;
  NSUInteger maxLength = textLength - location;
  return NSMakeRange(location, MIN((NSUInteger)length, maxLength));
}

- (void)addHighlightAtRange:(NSRange)range
                  textColor:(NSColor *)foregroundColor
            backgroundColor:(NSColor *)backgroundColor
                 toString:(NSMutableAttributedString *)content
{
  if (range.location == NSNotFound || range.length == 0) return;
  NSMutableDictionary<NSAttributedStringKey, id> *attributes = [NSMutableDictionary dictionary];
  if (foregroundColor != nil) attributes[NSForegroundColorAttributeName] = foregroundColor;
  if (backgroundColor != nil) attributes[NSBackgroundColorAttributeName] = backgroundColor;
  if (attributes.count > 0) [content addAttributes:attributes range:range];
}

- (void)refreshText
{
  if (_textView == nil) return;
  NSString *text = _text ?: @"";
  NSRange selectedRange = _textView.selectedRange;
  NSFont *font = [NSFont systemFontOfSize:_fontSize > 0 ? _fontSize : 14];
  NSDictionary *attributes = @{
    NSFontAttributeName: font,
    NSForegroundColorAttributeName: _textColor ?: NSColor.labelColor,
  };
  NSMutableAttributedString *content = nil;
  if (_cardMode) {
    if (@available(macOS 12.0, *)) {
      NSAttributedStringMarkdownParsingOptions *options = [NSAttributedStringMarkdownParsingOptions new];
      options.interpretedSyntax = NSAttributedStringMarkdownInterpretedSyntaxFull;
      options.failurePolicy = NSAttributedStringMarkdownParsingFailureReturnPartiallyParsedIfPossible;
      NSAttributedString *parsed = [[NSAttributedString alloc] initWithMarkdownString:text
                                                                              options:options
                                                                              baseURL:nil
                                                                                error:nil];
      if (parsed != nil) content = [parsed mutableCopy];
    }
  }
  if (content == nil) {
    content = [[NSMutableAttributedString alloc] initWithString:text attributes:attributes];
  } else if (content.length > 0) {
    NSRange fullRange = NSMakeRange(0, content.length);
    [content addAttribute:NSForegroundColorAttributeName
                    value:_textColor ?: NSColor.labelColor
                    range:fullRange];
    [content enumerateAttribute:NSFontAttributeName
                        inRange:fullRange
                        options:0
                     usingBlock:^(id value, NSRange range, BOOL *stop) {
      if (value == nil) [content addAttribute:NSFontAttributeName value:font range:range];
    }];
  }
  BOOL textChanged = ![_textView.string isEqualToString:content.string];
  NSRange cueRange = [self highlightRangeAtStart:_activeCueStart
                                           length:_activeCueLength
                                       textLength:text.length];
  [self addHighlightAtRange:cueRange
                  textColor:_activeCueTextColor
            backgroundColor:[_activeAccentColor colorWithAlphaComponent:(18.0 / 255.0)]
                    toString:content];
  // AI key-point marks draw as underlines so they coexist with the cue tint
  // instead of covering it. Applied before the active word, which keeps top
  // priority either way since it only touches the foreground color.
  NSColor *keyColor = _keyHighlightColor ?: NSColor.systemOrangeColor;
  for (NSValue *value in _keyRanges) {
    NSRange keyRange = [self highlightRangeAtStart:(NSInteger)value.rangeValue.location
                                            length:(NSInteger)value.rangeValue.length
                                        textLength:text.length];
    if (keyRange.location == NSNotFound || keyRange.length == 0) continue;
    [content addAttributes:@{
      NSUnderlineStyleAttributeName: @(NSUnderlineStyleSingle),
      NSUnderlineColorAttributeName: keyColor,
    } range:keyRange];
  }
  // Apply the word second so it has visual priority inside the cue highlight.
  NSRange wordRange = [self highlightRangeAtStart:_activeWordStart
                                            length:_activeWordLength
                                        textLength:text.length];
  [self addHighlightAtRange:wordRange
                  textColor:_activeWordTextColor
            backgroundColor:nil
                    toString:content];
  _textView.activeWordRange = wordRange;
  _textView.activeWordPillColor = _activeAccentColor;
  _textView.loopStartMarkerIndex = _loopStartMarkerIndex;
  _textView.loopStartMarkerAfter = _loopStartMarkerAfter;
  _textView.loopEndMarkerIndex = _loopEndMarkerIndex;
  _textView.loopEndMarkerAfter = _loopEndMarkerAfter;
  [_textView.textStorage setAttributedString:content];

  // Playback props are updated frequently.  Restoring an existing selection
  // keeps an in-progress or completed mouse selection usable for the context
  // menu instead of clearing it whenever the active word changes.
  if (!textChanged && selectedRange.location != NSNotFound && selectedRange.location < content.length) {
    NSUInteger length = MIN(selectedRange.length, content.length - selectedRange.location);
    _textView.selectedRange = NSMakeRange(selectedRange.location, length);
  }
}

- (void)scrollActiveCueIntoView
{
  NSRange range = [self highlightRangeAtStart:_activeCueStart
                                       length:_activeCueLength
                                   textLength:_textView.string.length];
  if (range.location != NSNotFound && range.length > 0) {
    [_textView scrollRangeToVisible:range];
  }
}

- (void)askSelectionFromTextView:(NSTextView *)textView
{
  NSRange range = textView.selectedRange;
  if (range.location == NSNotFound || range.length == 0 || NSMaxRange(range) > textView.string.length) return;
  NSString *selection = [[textView.string substringWithRange:range]
      stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
  RCTDirectEventBlock onAskSelection = self.onAskSelection;
  if (selection.length == 0 || onAskSelection == nil) return;

  // NSMenu invokes its target while its tracking loop is active.  Do not
  // synchronously enter React from that action: the event can render or
  // unmount this view before AppKit has finished closing the menu.  Capturing
  // both values and delivering in the default run-loop mode runs it after
  // menu tracking has unwound.
  CFRunLoopRef mainRunLoop = CFRunLoopGetMain();
  CFRunLoopPerformBlock(mainRunLoop, kCFRunLoopDefaultMode, ^{
    onAskSelection(@{ @"text": selection });
  });
  CFRunLoopWakeUp(mainRunLoop);
}

- (void)tapAtCharacterIndex:(NSUInteger)index
{
  if (self.onTapAtCharacter != nil) {
    self.onTapAtCharacter(@{ @"index": @(index) });
  }
}

@end

@interface RNSelectableSubtitleViewManager : RCTViewManager
@end

@implementation RNSelectableSubtitleViewManager

RCT_EXPORT_MODULE(RNSelectableSubtitleView);

+ (BOOL)requiresMainQueueSetup
{
  return YES;
}

- (NSView *)view
{
  return [RNSelectableSubtitleView new];
}

RCT_EXPORT_VIEW_PROPERTY(text, NSString)
RCT_EXPORT_VIEW_PROPERTY(fontSize, CGFloat)
RCT_EXPORT_VIEW_PROPERTY(selectionEnabled, BOOL)
RCT_EXPORT_VIEW_PROPERTY(textColor, NSColor)
RCT_EXPORT_VIEW_PROPERTY(activeCueStart, NSInteger)
RCT_EXPORT_VIEW_PROPERTY(activeCueLength, NSInteger)
RCT_EXPORT_VIEW_PROPERTY(activeWordStart, NSInteger)
RCT_EXPORT_VIEW_PROPERTY(activeWordLength, NSInteger)
RCT_EXPORT_VIEW_PROPERTY(activeCueTextColor, NSColor)
RCT_EXPORT_VIEW_PROPERTY(activeAccentColor, NSColor)
RCT_EXPORT_VIEW_PROPERTY(activeWordTextColor, NSColor)
RCT_EXPORT_VIEW_PROPERTY(keyRangesJson, NSString)
RCT_EXPORT_VIEW_PROPERTY(keyHighlightColor, NSColor)
RCT_EXPORT_VIEW_PROPERTY(loopStartMarkerIndex, NSInteger)
RCT_EXPORT_VIEW_PROPERTY(loopStartMarkerAfter, BOOL)
RCT_EXPORT_VIEW_PROPERTY(loopEndMarkerIndex, NSInteger)
RCT_EXPORT_VIEW_PROPERTY(loopEndMarkerAfter, BOOL)
RCT_EXPORT_VIEW_PROPERTY(onAskSelection, RCTDirectEventBlock)
RCT_EXPORT_VIEW_PROPERTY(onTapAtCharacter, RCTDirectEventBlock)
RCT_EXPORT_VIEW_PROPERTY(onScrollPositionChange, RCTDirectEventBlock)
RCT_EXPORT_VIEW_PROPERTY(cardMode, BOOL)
RCT_EXPORT_VIEW_PROPERTY(contentOffsetY, CGFloat)

@end

// Keyboard events are handed to JS via promise resolution (main thread safe)
// instead of direct RCTResponseSenderBlock callbacks, which must run on the
// JS thread and abort under TurboModules when invoked from AppKit's main thread.
@interface RNKeyboard : NSObject <RCTBridgeModule>
@property (nonatomic, copy) RCTPromiseResolveBlock keyResolve;
@property (nonatomic, copy) RCTPromiseRejectBlock keyReject;
@property (nonatomic, strong) NSMutableArray<NSString *> *pendingKeys;
@property (nonatomic, strong) id keyMonitor;
@end

@implementation RNKeyboard

RCT_EXPORT_MODULE(RNKeyboard);

RCT_EXPORT_METHOD(startListening)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    if (self.keyMonitor != nil) return;
    __weak RNKeyboard *weakSelf = self;
    self.keyMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown
                                                            handler:^(NSEvent *event) {
      __strong RNKeyboard *strongSelf = weakSelf;
      if (strongSelf == nil) return event;

      // Never steal navigation or Space from an editable AppKit field.  This
      // includes the native AI-question prompt and any normal text editor.
      NSResponder *firstResponder = event.window.firstResponder;
      if ([firstResponder isKindOfClass:[NSTextView class]] &&
          ((NSTextView *)firstResponder).isEditable) {
        return event;
      }

      // Only capture plain presses (no ⌘/⌃/⌥) for keys the flashcards use, so
      // system shortcuts keep working. Consuming handled keys (returning nil)
      // also stops macOS from beeping on unhandled key events.
      NSEventModifierFlags flags = event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
      if (flags & (NSEventModifierFlagCommand | NSEventModifierFlagControl | NSEventModifierFlagOption)) {
        return event;
      }

      NSString *key = nil;
      switch (event.keyCode) {
        case 49:  if (event.isARepeat) return (NSEvent *)nil;
                  key = @" "; break;           // Space
        case 36:
        case 76:  if (event.isARepeat) return (NSEvent *)nil;
                  key = @"Enter"; break;       // Return / keypad Enter
        case 123: key = @"ArrowLeft"; break;   // ←
        case 124: key = @"ArrowRight"; break;  // →
        case 125: key = @"ArrowDown"; break;   // ↓
        case 126: key = @"ArrowUp"; break;     // ↑
        case 18:  key = @"1"; break;           // 评分：忘了
        case 19:  key = @"2"; break;           // 评分：模糊
        case 20:  key = @"3"; break;           // 评分：认识
        case 0:   if (event.isARepeat) return (NSEvent *)nil;
                  key = @"a"; break;           // 随便学学：上一条
        case 2:   if (event.isARepeat) return (NSEvent *)nil;
                  key = @"d"; break;           // 随便学学：下一条
        case 35:  if (event.isARepeat) return (NSEvent *)nil;
                  key = @"p"; break;           // 随便学学：暂停 / 继续
        default: return event;
      }

      [strongSelf dispatchKey:key];
      return (NSEvent *)nil; // consume the event: no responder-chain fallthrough, no system beep
    }];
  });
}

// The listening player supports Space plus short forward/backward seeking.
RCT_EXPORT_METHOD(startPlaybackListening)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    if (self.keyMonitor != nil) return;
    __weak RNKeyboard *weakSelf = self;
    self.keyMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown
                                                            handler:^(NSEvent *event) {
      __strong RNKeyboard *strongSelf = weakSelf;
      if (strongSelf == nil) return event;

      // A prompt/editor owns its own Space and arrow keys.  Outside editable
      // fields, the non-editable subtitle surface continues to use playback
      // shortcuts as before.
      NSResponder *firstResponder = event.window.firstResponder;
      if ([firstResponder isKindOfClass:[NSTextView class]] &&
          ((NSTextView *)firstResponder).isEditable) {
        return event;
      }

      NSEventModifierFlags flags = event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
      if (flags & (NSEventModifierFlagCommand | NSEventModifierFlagControl | NSEventModifierFlagOption)) {
        return event;
      }

      NSString *key = nil;
      switch (event.keyCode) {
        case 49:  key = @" "; break;           // Space
        case 123: key = @"ArrowLeft"; break;   // ←
        case 124: key = @"ArrowRight"; break;  // →
        default: return event;
      }

      [strongSelf dispatchKey:key];
      return (NSEvent *)nil;
    }];
  });
}

RCT_EXPORT_METHOD(getNextKey:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    self.keyResolve = resolve;
    self.keyReject = reject;
    [self flushPendingKey];
  });
}

RCT_EXPORT_METHOD(stopListening)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    if (self.keyMonitor != nil) {
      [NSEvent removeMonitor:self.keyMonitor];
      self.keyMonitor = nil;
    }
    // Release a pending JS waiter so its pump loop can exit.
    if (self.keyResolve != nil) {
      RCTPromiseResolveBlock resolve = self.keyResolve;
      self.keyResolve = nil;
      self.keyReject = nil;
      resolve((NSString *)nil);
    }
    [self.pendingKeys removeAllObjects];
  });
}

- (void)dispatchKey:(NSString *)key
{
  if (self.keyResolve == nil) {
    if (self.pendingKeys == nil) self.pendingKeys = [NSMutableArray array];
    if (self.pendingKeys.count < 128) [self.pendingKeys addObject:key];
    return;
  }
  RCTPromiseResolveBlock resolve = self.keyResolve;
  self.keyResolve = nil;
  self.keyReject = nil;
  resolve(key);
}

- (void)flushPendingKey
{
  if (self.keyResolve == nil || self.pendingKeys.count == 0) return;
  NSString *key = self.pendingKeys.firstObject;
  [self.pendingKeys removeObjectAtIndex:0];
  RCTPromiseResolveBlock resolve = self.keyResolve;
  self.keyResolve = nil;
  self.keyReject = nil;
  resolve(key);
}

@end

// A selectable React Native <Text> is backed by an NSTextView on macOS.  The
// default menu only knows how to copy, so add the listening-specific actions
// without putting extra buttons in the study UI.
@interface RNSelectedTextMenu : NSObject <RCTBridgeModule>
@property (nonatomic, copy) RCTPromiseResolveBlock selectionResolve;
@property (nonatomic, copy) RCTPromiseRejectBlock selectionReject;
@property (nonatomic, strong) NSMutableArray<NSString *> *pendingSelections;
@property (nonatomic, strong) id mouseMonitor;
@property (nonatomic, copy) NSString *menuSelection;
- (void)copySelection:(id)sender;
- (void)askSelection:(id)sender;
- (void)dispatchSelection:(NSString *)selection;
- (void)flushPendingSelection;
- (NSTextView *)subtitleTextViewAtEvent:(NSEvent *)event;
@end

@implementation RNSelectedTextMenu

RCT_EXPORT_MODULE(RNSelectedTextMenu);

RCT_EXPORT_METHOD(startListening)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    if (self.mouseMonitor != nil) return;

    __weak RNSelectedTextMenu *weakSelf = self;
    self.mouseMonitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskRightMouseDown
                                                               handler:^NSEvent *(NSEvent *event) {
      __strong RNSelectedTextMenu *strongSelf = weakSelf;
      if (strongSelf == nil) return event;

      NSTextView *textView = [strongSelf subtitleTextViewAtEvent:event];
      if (textView == nil) return event;
      // Normal TextInput controls are editable. The listening screen only
      // exposes one non-editable selectable text view: the subtitle stream.
      // Avoid comparing its rendered string, because React Native's nested
      // Text spans can alter that string while preserving the visible words.
      if (textView.isEditable || !textView.isSelectable) {
        return event;
      }
      NSRange range = textView.selectedRange;
      if (range.location == NSNotFound || range.length == 0 || NSMaxRange(range) > textView.string.length) {
        return event;
      }

      NSString *selection = [[textView.string substringWithRange:range]
          stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
      if (selection.length == 0) return event;

      strongSelf.menuSelection = selection;
      NSMenu *menu = [[NSMenu alloc] initWithTitle:@"字幕操作"];
      NSMenuItem *copyItem = [[NSMenuItem alloc] initWithTitle:@"复制字幕"
                                                         action:@selector(copySelection:)
                                                  keyEquivalent:@""];
      copyItem.target = strongSelf;
      [menu addItem:copyItem];

      NSMenuItem *askItem = [[NSMenuItem alloc] initWithTitle:@"向 AI 提问"
                                                        action:@selector(askSelection:)
                                                 keyEquivalent:@""];
      askItem.target = strongSelf;
      [menu addItem:askItem];

      [NSMenu popUpContextMenu:menu withEvent:event forView:textView];
      return (NSEvent *)nil;
    }];
  });
}

- (NSTextView *)subtitleTextViewAtEvent:(NSEvent *)event
{
  NSView *contentView = event.window.contentView;
  if (contentView == nil) return nil;

  NSPoint point = [contentView convertPoint:event.locationInWindow fromView:nil];
  NSView *view = [contentView hitTest:point];
  Class rctTextViewClass = NSClassFromString(@"RCTTextView");
  while (view != nil && (rctTextViewClass == nil || ![view isKindOfClass:rctTextViewClass])) {
    view = view.superview;
  }
  if (view == nil) return nil;

  for (NSView *subview in view.subviews) {
    if ([subview isKindOfClass:[NSTextView class]]) {
      return (NSTextView *)subview;
    }
  }
  return nil;
}

RCT_EXPORT_METHOD(getNextSelection:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    self.selectionResolve = resolve;
    self.selectionReject = reject;
    [self flushPendingSelection];
  });
}

RCT_EXPORT_METHOD(stopListening)
{
  dispatch_async(dispatch_get_main_queue(), ^{
    if (self.mouseMonitor != nil) {
      [NSEvent removeMonitor:self.mouseMonitor];
      self.mouseMonitor = nil;
    }
    self.menuSelection = nil;
    if (self.selectionResolve != nil) {
      RCTPromiseResolveBlock resolve = self.selectionResolve;
      self.selectionResolve = nil;
      self.selectionReject = nil;
      resolve((NSString *)nil);
    }
    [self.pendingSelections removeAllObjects];
  });
}

- (void)copySelection:(id)sender
{
  if (self.menuSelection.length > 0) {
    NSPasteboard *pasteboard = NSPasteboard.generalPasteboard;
    [pasteboard clearContents];
    [pasteboard setString:self.menuSelection forType:NSPasteboardTypeString];
  }
  self.menuSelection = nil;
}

- (void)askSelection:(id)sender
{
  NSString *selection = self.menuSelection;
  self.menuSelection = nil;
  if (selection.length > 0) {
    [self dispatchSelection:selection];
  }
}

- (void)dispatchSelection:(NSString *)selection
{
  if (self.selectionResolve == nil) {
    if (self.pendingSelections == nil) self.pendingSelections = [NSMutableArray array];
    if (self.pendingSelections.count < 8) [self.pendingSelections addObject:selection];
    return;
  }
  RCTPromiseResolveBlock resolve = self.selectionResolve;
  self.selectionResolve = nil;
  self.selectionReject = nil;
  resolve(selection);
}

- (void)flushPendingSelection
{
  if (self.selectionResolve == nil || self.pendingSelections.count == 0) return;
  NSString *selection = self.pendingSelections.firstObject;
  [self.pendingSelections removeObjectAtIndex:0];
  RCTPromiseResolveBlock resolve = self.selectionResolve;
  self.selectionResolve = nil;
  self.selectionReject = nil;
  resolve(selection);
}

@end

@implementation AppDelegate

- (void)applicationDidFinishLaunching:(NSNotification *)notification
{
#if DEBUG
  // react-native-macos can leave its bundle progress NSPanel visible at
  // "Downloading 100%" even after Metro returned the complete bundle. The
  // panel is cosmetic; disabling it does not change bundle loading or errors.
  RCTDevLoadingViewSetEnabled(NO);
#endif
  self.moduleName = @"clarora";
  // You can add your custom initial props in the dictionary below.
  // They will be passed down to the ViewController used by React Native.
  self.initialProps = @{};

  [super applicationDidFinishLaunching:notification];

  // LaunchServices can keep serving a stale icon for development builds that
  // reuse the same bundle identifier. Load the current bundled artwork and
  // set the Dock image explicitly so it always matches this build.
  NSURL *iconURL = [NSBundle.mainBundle URLForResource:@"AppIcon" withExtension:@"icns"];
  NSImage *bundledIcon = iconURL == nil ? nil : [[NSImage alloc] initWithContentsOfURL:iconURL];
  if (bundledIcon != nil) {
    const CGFloat iconSide = 512.0;
    NSImage *dockIcon = [[NSImage alloc] initWithSize:NSMakeSize(iconSide, iconSide)];
    [dockIcon lockFocus];
    [NSGraphicsContext saveGraphicsState];
    [[NSBezierPath bezierPathWithRoundedRect:NSMakeRect(0, 0, iconSide, iconSide)
                                     xRadius:116.0
                                     yRadius:116.0] addClip];
    [bundledIcon drawInRect:NSMakeRect(0, 0, iconSide, iconSide)
                   fromRect:NSZeroRect
                  operation:NSCompositingOperationCopy
                   fraction:1.0
             respectFlipped:NO
                      hints:@{NSImageHintInterpolation: @(NSImageInterpolationHigh)}];
    [NSGraphicsContext restoreGraphicsState];
    [dockIcon unlockFocus];
    NSApp.applicationIconImage = dockIcon;
  }

  // Keep the React Native window alive for the lifetime of the app. Closing
  // and recreating it triggers an invalid release inside this RN macOS build.
  self.claroraMainWindow = NSApp.windows.firstObject;
  self.claroraMainWindow.delegate = self;
}

// Treat the red close button as "hide". This preserves the React Native root
// view and lets the Dock icon restore the same window safely.
- (BOOL)windowShouldClose:(NSWindow *)window
{
  [window orderOut:self];
  return NO;
}

- (BOOL)applicationShouldHandleReopen:(NSApplication *)sender
                    hasVisibleWindows:(BOOL)hasVisibleWindows
{
  if (hasVisibleWindows) return YES;

  NSWindow *window = self.claroraMainWindow;
  if (window != nil) {
    if (window.isMiniaturized) {
      [window deminiaturize:self];
    }
    [window makeKeyAndOrderFront:self];
  }

  return YES;
}

- (NSURL *)bundleURL
{
#if DEBUG
  return [NSURL URLWithString:@"http://127.0.0.1:8081/index.bundle?platform=macos&dev=true&minify=false"];
#else
  return [[NSBundle mainBundle] URLForResource:@"main" withExtension:@"jsbundle"];
#endif
}

- (NSURL *)sourceURLForBridge:(RCTBridge *)bridge
{
  return [self bundleURL];
}

/// This method controls whether the `concurrentRoot`feature of React18 is turned on or off.
///
/// @see: https://reactjs.org/blog/2022/03/29/react-v18.html
/// @note: This requires to be rendering on Fabric (i.e. on the New Architecture).
/// @return: `true` if the `concurrentRoot` feature is enabled. Otherwise, it returns `false`.
- (BOOL)concurrentRootEnabled
{
#ifdef RN_FABRIC_ENABLED
  return true;
#else
  return false;
#endif
}

@end

// Runs a shell command in the app's host environment. Used by the listening
// screen's "命令导入" to import audio paths printed by a user-configured
// command (e.g. lsof on a media player app).
@interface RNShell : NSObject <RCTBridgeModule>
@end

@implementation RNShell

RCT_EXPORT_MODULE(RNShell);

RCT_EXPORT_METHOD(runCommand:(NSString *)command
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
    NSTask *task = [[NSTask alloc] init];
    task.launchPath = @"/bin/zsh";
    task.arguments = @[@"-c", command];
    NSPipe *outputPipe = [NSPipe pipe];
    NSPipe *errorPipe = [NSPipe pipe];
    task.standardOutput = outputPipe;
    task.standardError = errorPipe;

    @try {
      [task launch];
    } @catch (NSException *exception) {
      dispatch_async(dispatch_get_main_queue(), ^{
        reject(@"shell_launch_error", exception.reason ?: @"无法启动命令", nil);
      });
      return;
    }
    [task waitUntilExit];

    NSData *outputData = [[outputPipe fileHandleForReading] readDataToEndOfFile];
    NSString *output = [[NSString alloc] initWithData:outputData
                                             encoding:NSUTF8StringEncoding];
    if (output == nil) {
      output = @"";
    }

    dispatch_async(dispatch_get_main_queue(), ^{
      if (task.terminationStatus != 0 && output.length == 0) {
        NSData *errorData = [[errorPipe fileHandleForReading] readDataToEndOfFile];
        NSString *stderrText = [[NSString alloc] initWithData:errorData
                                                     encoding:NSUTF8StringEncoding];
        reject(@"shell_error",
               [NSString stringWithFormat:@"命令退出码 %d：%@",
                task.terminationStatus,
                stderrText.length ? stderrText : @"无输出"],
               nil);
        return;
      }
      resolve(output);
    });
  });
}

// Run an executable directly with an argument array (no shell parsing) —
// used for multipart uploads where shell quoting is error-prone.
RCT_EXPORT_METHOD(runArgs:(NSString *)launchPath
                  arguments:(NSArray *)arguments
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
    NSTask *task = [[NSTask alloc] init];
    task.launchPath = launchPath;
    task.arguments = arguments;
    NSPipe *outputPipe = [NSPipe pipe];
    NSPipe *errorPipe = [NSPipe pipe];
    task.standardOutput = outputPipe;
    task.standardError = errorPipe;

    @try {
      [task launch];
    } @catch (NSException *exception) {
      dispatch_async(dispatch_get_main_queue(), ^{
        reject(@"shell_launch_error", exception.reason ?: @"无法启动命令", nil);
      });
      return;
    }
    [task waitUntilExit];

    NSData *outputData = [[outputPipe fileHandleForReading] readDataToEndOfFile];
    NSString *output = [[NSString alloc] initWithData:outputData
                                             encoding:NSUTF8StringEncoding];
    if (output == nil) {
      output = @"";
    }

    dispatch_async(dispatch_get_main_queue(), ^{
      if (task.terminationStatus != 0 && output.length == 0) {
        NSData *errorData = [[errorPipe fileHandleForReading] readDataToEndOfFile];
        NSString *stderrText = [[NSString alloc] initWithData:errorData
                                                     encoding:NSUTF8StringEncoding];
        reject(@"shell_error",
               [NSString stringWithFormat:@"命令退出码 %d：%@",
                task.terminationStatus,
                stderrText.length ? stderrText : @"无输出"],
               nil);
        return;
      }
      resolve(output);
    });
  });
}

@end

// ── Mac microphone recorder for speaking practice (AAC m4a, ASR-friendly) ──

@interface RNMacAudioRecorder : NSObject <RCTBridgeModule>
@property (nonatomic, strong) AVAudioRecorder *recorder;
@property (nonatomic, copy) NSString *recordingPath;
@end

@implementation RNMacAudioRecorder

RCT_EXPORT_MODULE(RNMacAudioRecorder)

RCT_EXPORT_METHOD(startRecording:(RCTPromiseResolveBlock)resolve
                 rejecter:(RCTPromiseRejectBlock)reject)
{
  // 先确认系统里有可用的麦克风输入设备，缺失时 record 会无声失败。
  AVCaptureDevice *microphone = [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeAudio];
  if (microphone == nil) {
    reject(@"no_mic", @"没有检测到麦克风输入设备，请检查系统声音设置", nil);
    return;
  }
  [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio
                            completionHandler:^(BOOL granted) {
    if (!granted) {
      reject(@"mic_denied", @"麦克风权限被拒绝，请在系统设置中允许", nil);
      return;
    }
    dispatch_async(dispatch_get_main_queue(), ^{
      NSString *base = [NSSearchPathForDirectoriesInDomains(NSApplicationSupportDirectory, NSUserDomainMask, YES) firstObject];
      NSString *dir = [base stringByAppendingPathComponent:@"Clarora/Recordings"];
      [[NSFileManager defaultManager] createDirectoryAtPath:dir withIntermediateDirectories:YES attributes:nil error:nil];
      NSString *path = [dir stringByAppendingPathComponent:
        [NSString stringWithFormat:@"rec_%lld.m4a", (long long)([NSDate date].timeIntervalSince1970 * 1000)]];

      // 不同输入设备对采样率的支持差异会导致启动失败，逐档重试。
      NSString *failure = @"无法开始录音";
      for (NSNumber *rate in @[@44100, @48000, @16000]) {
        NSDictionary *settings = @{
          AVFormatIDKey: @(kAudioFormatMPEG4AAC),
          AVSampleRateKey: rate,
          AVNumberOfChannelsKey: @1,
          AVEncoderBitRateKey: @96000,
        };
        NSError *initError = nil;
        self.recorder = [[AVAudioRecorder alloc] initWithURL:[NSURL fileURLWithPath:path]
                                                    settings:settings
                                                       error:&initError];
        if (self.recorder == nil || ![self.recorder prepareToRecord]) {
          failure = [NSString stringWithFormat:@"初始化失败（%@ Hz）：%@", rate, initError.localizedDescription ?: @"prepareToRecord 失败"];
          [self.recorder stop];
          self.recorder = nil;
          continue;
        }
        if ([self.recorder record]) {
          self.recordingPath = path;
          resolve(nil);
          return;
        }
        failure = [NSString stringWithFormat:@"启动录音失败（%@ Hz）", rate];
        [self.recorder stop];
        self.recorder = nil;
      }
      reject(@"record_failed", failure, nil);
    });
  }];
}

RCT_EXPORT_METHOD(stopRecording:(RCTPromiseResolveBlock)resolve
                 rejecter:(RCTPromiseRejectBlock)reject)
{
  if (self.recorder == nil) {
    reject(@"not_recording", @"当前没有录音", nil);
    return;
  }
  [self.recorder stop];
  self.recorder = nil;
  NSString *path = self.recordingPath ?: @"";
  NSString *name = [path lastPathComponent] ?: @"";
  resolve(@{ @"uri": [@"file://" stringByAppendingString:path], @"name": name });
}

// 番茄钟阶段切换提示音（系统 Glass 音效，无需音频文件）。
RCT_EXPORT_METHOD(playChime:(RCTPromiseResolveBlock)resolve
                 rejecter:(RCTPromiseRejectBlock)reject)
{
  NSSound *sound = [NSSound soundNamed:@"Glass"];
  if (sound == nil) {
    resolve(@NO);
    return;
  }
  [sound play];
  resolve(@YES);
}

@end

// ── 休息音乐专用播放器(独立于 RNMacAudio 的共享 player,互不打断)──────

@interface RNBreakAudioModule : NSObject <RCTBridgeModule>
@property (nonatomic, strong) AVAudioPlayer *player;
@end

@implementation RNBreakAudioModule

RCT_EXPORT_MODULE(RNBreakAudio)

RCT_EXPORT_METHOD(play:(NSString *)path)
{
  NSString *clean = [path stringByReplacingOccurrencesOfString:@"file://" withString:@""];
  self.player = [[AVAudioPlayer alloc] initWithContentsOfURL:[NSURL fileURLWithPath:clean] error:nil];
  self.player.numberOfLoops = -1;
  [self.player play];
}

RCT_EXPORT_METHOD(pause)
{
  [self.player pause];
}

RCT_EXPORT_METHOD(resume)
{
  [self.player play];
}

RCT_EXPORT_METHOD(stop)
{
  [self.player stop];
  self.player = nil;
}

@end
