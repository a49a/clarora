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

// ── RNMacWhisper：端侧字幕转写（whisper.cpp + 本地 ggml 模型，离线生成）──────
// 依赖 Homebrew 的 whisper-cpp（brew install whisper-cpp），库路径与 libmpv
// 相同：/opt/homebrew。音频经 AVFoundation 流式解码为 16 kHz 单声道 Float32。

#import <whisper.h>
#import <ggml-backend.h>
#include <vector>

@interface RNMacWhisper : NSObject <RCTBridgeModule>
@end

@implementation RNMacWhisper

RCT_EXPORT_MODULE(RNMacWhisper);

+ (BOOL)requiresMainQueueSetup { return NO; }

// whisper.cpp 1.9 的计算后端（CPU / Metal）是运行时动态加载的模块；应用进程
// 必须显式加载，否则无 GPU 设备时 use_gpu=true 会直接 GGML_ABORT 崩掉进程。
// Homebrew 的后端位于 ggml 的 libexec 目录（/opt/homebrew/opt/ggml 为版本无关符号链接）。
static void LoadWhisperBackends(void) {
  static dispatch_once_t once;
  dispatch_once(&once, ^{
    ggml_backend_load_all_from_path("/opt/homebrew/opt/ggml/libexec");
    if (ggml_backend_dev_count() == 0) ggml_backend_load_all();
  });
}

// One loaded model per path; repeat transcriptions skip the model load.
static NSMutableDictionary<NSString *, NSValue *> *WhisperContextCache(void) {
  static NSMutableDictionary<NSString *, NSValue *> *contexts = nil;
  static dispatch_once_t once;
  dispatch_once(&once, ^{ contexts = [NSMutableDictionary dictionary]; });
  return contexts;
}

- (dispatch_queue_t)transcriptionQueue {
  static dispatch_queue_t queue = nil;
  static dispatch_once_t once;
  dispatch_once(&once, ^{ queue = dispatch_queue_create("app.clarora.whisper", DISPATCH_QUEUE_SERIAL); });
  return queue;
}

// Stream-decode any AVFoundation-readable recording into 16 kHz mono Float32
// with bounded memory. Shared by every on-device ASR engine (whisper.cpp and
// sherpa-onnx accept exactly this input format).
static BOOL ClaroraDecodeAudio16kMono(NSString *path,
                                      std::vector<float> *samples,
                                      NSString **errorText) {
  NSError *error = nil;
  AVAudioFile *file = [[AVAudioFile alloc] initForReading:[NSURL fileURLWithPath:path] error:&error];
  if (file == nil || file.processingFormat == nil) {
    *errorText = error.localizedDescription ?: @"无法读取音频文件";
    return NO;
  }
  if (file.length / file.processingFormat.sampleRate > 4 * 3600.0) {
    *errorText = @"音频超过 4 小时，请分段后生成字幕";
    return NO;
  }
  AVAudioFormat *target = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:16000 channels:1];
  AVAudioConverter *converter = [[AVAudioConverter alloc] initFromFormat:file.processingFormat toFormat:target];
  if (converter == nil) {
    *errorText = @"该音频格式不受支持（OGG 不支持，可先转 MP3/M4A/WAV）";
    return NO;
  }

  AVAudioPCMBuffer *input = [[AVAudioPCMBuffer alloc] initWithPCMFormat:file.processingFormat
                                                          frameCapacity:8192];
  AVAudioPCMBuffer *output = [[AVAudioPCMBuffer alloc] initWithPCMFormat:target frameCapacity:16384];
  __block BOOL endOfFile = NO;
  while (true) {
    AVAudioConverterOutputStatus status = [converter convertToBuffer:output error:&error
                                                  withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount packets, AVAudioConverterInputStatus *inputStatus) {
      if (endOfFile) { *inputStatus = AVAudioConverterInputStatus_EndOfStream; return nil; }
      if ([file readIntoBuffer:input error:nil] == 0) {
        endOfFile = YES;
        *inputStatus = AVAudioConverterInputStatus_EndOfStream;
        return nil;
      }
      *inputStatus = AVAudioConverterInputStatus_HaveData;
      return input;
    }];
    if (status == AVAudioConverterOutputStatus_HaveData) {
      float *channel = output.floatChannelData[0];
      samples->insert(samples->end(), channel, channel + output.frameLength);
      continue;
    }
    if (status == AVAudioConverterOutputStatus_EndOfStream) break;
    *errorText = error.localizedDescription ?: @"音频转码失败";
    return NO;
  }
  if (samples->empty()) {
    *errorText = @"音频内容为空";
    return NO;
  }
  return YES;
}

RCT_EXPORT_METHOD(transcribe:(NSString *)audioPath
                  modelPath:(NSString *)modelPath
                  language:(NSString *)language
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(self.transcriptionQueue, ^{
    std::vector<float> samples;
    NSString *errorText = nil;
    if (!ClaroraDecodeAudio16kMono(audioPath, &samples, &errorText)) {
      reject(@"decode_error", errorText, nil);
      return;
    }

    NSValue *cached = WhisperContextCache()[modelPath];
    whisper_context *context = cached ? (whisper_context *)cached.pointerValue : nil;
    if (context == nil) {
      LoadWhisperBackends();
      struct whisper_context_params contextParams = whisper_context_default_params();
      // 仅在确认存在计算设备时启用 GPU（Metal），否则退回 CPU，避免 GGML 断言崩溃。
      contextParams.use_gpu = ggml_backend_dev_count() > 0;
      context = whisper_init_from_file_with_params(modelPath.UTF8String, contextParams);
      if (context == nil) {
        reject(@"model_error", @"无法加载端侧模型文件，请到设置中重新下载", nil);
        return;
      }
      WhisperContextCache()[modelPath] = [NSValue valueWithPointer:context];
    }

    struct whisper_full_params params = whisper_full_default_params(WHISPER_SAMPLING_GREEDY);
    params.print_progress = false;
    params.print_special = false;
    params.print_realtime = false;
    params.print_timestamps = false;
    params.translate = false;
    params.language = language.length ? language.UTF8String : "auto";

    if (whisper_full(context, params, samples.data(), (int)samples.size()) != 0) {
      reject(@"transcribe_error", @"端侧转写失败，请重试或更换模型", nil);
      return;
    }

    double duration = (double)samples.size() / 16000.0;
    NSMutableArray<NSDictionary *> *segments = [NSMutableArray array];
    int count = whisper_full_n_segments(context);
    for (int i = 0; i < count; i++) {
      double start = whisper_full_get_segment_t0(context, i) / 100.0;  // whisper 时刻单位是 10ms
      double end = MAX(whisper_full_get_segment_t1(context, i) / 100.0, start);
      NSString *text = [NSString stringWithUTF8String:whisper_full_get_segment_text(context, i)];
      [segments addObject:@{ @"start": @(start), @"end": @(end), @"text": text }];
    }
    resolve(@{ @"segments": segments, @"duration": @(duration) });
  });
}

@end

// ── RNMacSenseVoice：端侧字幕转写（sherpa-onnx + SenseVoiceSmall ONNX）──────
// 依赖 sherpa-onnx 动态库（libsherpa-onnx-c-api + onnxruntime，见 README），
// 模型为 SenseVoiceSmall int8，支持中/英/日/韩/粤，自动检测语言。结果只有逐
// token 时间戳，这里按标点和停顿聚合成字幕段。

#import <sherpa-onnx/c-api/c-api.h>

@interface RNMacSenseVoice : NSObject <RCTBridgeModule>
@end

@implementation RNMacSenseVoice

RCT_EXPORT_MODULE(RNMacSenseVoice);

+ (BOOL)requiresMainQueueSetup { return NO; }

- (dispatch_queue_t)transcriptionQueue {
  static dispatch_queue_t queue = nil;
  static dispatch_once_t once;
  dispatch_once(&once, ^{ queue = dispatch_queue_create("app.clarora.sensevoice", DISPATCH_QUEUE_SERIAL); });
  return queue;
}

// One loaded recognizer per model path; model load dominates latency otherwise.
static NSMutableDictionary<NSString *, NSValue *> *SenseVoiceRecognizers(void) {
  static NSMutableDictionary<NSString *, NSValue *> *recognizers = nil;
  static dispatch_once_t once;
  dispatch_once(&once, ^{ recognizers = [NSMutableDictionary dictionary]; });
  return recognizers;
}

// SenseVoice 只输出逐 token 时间戳；按标点、停顿和长度聚合成适合跟读的字幕段。
+ (NSMutableArray<NSDictionary *> *)cuesFromResult:(const SherpaOnnxOfflineRecognizerResult *)result
                                         duration:(double)duration {
  NSMutableArray<NSDictionary *> *segments = [NSMutableArray array];
  if (result->tokens_arr == nil || result->timestamps == nil) {
    NSString *text = [NSString stringWithUTF8String:result->text ? result->text : ""];
    if (text.length) [segments addObject:@{ @"start": @(0), @"end": @(duration), @"text": text }];
    return segments;
  }

  NSMutableCharacterSet *punctuation = [NSMutableCharacterSet characterSetWithCharactersInString:@"。！？!?.,;:;:、，"];
  NSMutableString *cueText = [NSMutableString string];
  double cueStart = -1, prevEnd = -1;
  int32_t tokensInCue = 0;
  for (int32_t i = 0; i < result->count; i++) {
    NSString *token = [NSString stringWithUTF8String:result->tokens_arr[i]];
    if (token.length == 0 || [token hasPrefix:@"<|"]) continue;  // 语言/情感等特殊标记
    double timestamp = result->timestamps[i];
    if (timestamp < 0 || timestamp > duration) continue;
    if (cueStart < 0) cueStart = MAX(0, timestamp - 0.05);
    if (prevEnd >= 0 && timestamp - prevEnd > 0.6 && cueText.length > 0) {
      [segments addObject:@{ @"start": @(cueStart), @"end": @(MIN(prevEnd + 0.2, duration)), @"text": [cueText copy] }];
      [cueText setString:@""];
      cueStart = MAX(0, timestamp - 0.05);
      tokensInCue = 0;
    }
    [cueText appendString:token];
    tokensInCue++;
    prevEnd = timestamp;
    if ([token rangeOfCharacterFromSet:punctuation].location != NSNotFound || tokensInCue >= 20 || timestamp - cueStart >= 6.0) {
      [segments addObject:@{ @"start": @(cueStart), @"end": @(MIN(timestamp + 0.2, duration)), @"text": [cueText copy] }];
      [cueText setString:@""];
      cueStart = -1;
      tokensInCue = 0;
    }
  }
  if (cueText.length > 0) {
    [segments addObject:@{ @"start": @(MAX(cueStart, 0)), @"end": @(MIN(prevEnd + 0.2, duration)), @"text": [cueText copy] }];
  }
  return segments;
}

RCT_EXPORT_METHOD(transcribe:(NSString *)audioPath
                  modelPath:(NSString *)modelPath
                  tokensPath:(NSString *)tokensPath
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(self.transcriptionQueue, ^{
    std::vector<float> samples;
    NSString *errorText = nil;
    if (!ClaroraDecodeAudio16kMono(audioPath, &samples, &errorText)) {
      reject(@"decode_error", errorText, nil);
      return;
    }

    NSValue *cached = SenseVoiceRecognizers()[modelPath];
    const SherpaOnnxOfflineRecognizer *recognizer = cached ? (const SherpaOnnxOfflineRecognizer *)cached.pointerValue : nil;
    if (recognizer == nil) {
      SherpaOnnxOfflineRecognizerConfig config;
      memset(&config, 0, sizeof(config));
      config.feat_config.sample_rate = 16000;
      config.feat_config.feature_dim = 80;
      config.model_config.sense_voice.model = modelPath.UTF8String;
      config.model_config.sense_voice.language = "auto";  // zh/en/ja/ko/yue 自动检测
      config.model_config.sense_voice.use_itn = 1;
      config.model_config.tokens = tokensPath.UTF8String;
      config.model_config.num_threads = 2;
      recognizer = SherpaOnnxCreateOfflineRecognizer(&config);
      if (recognizer == nil) {
        reject(@"model_error", @"无法加载 SenseVoice 模型，请到设置中重新下载", nil);
        return;
      }
      SenseVoiceRecognizers()[modelPath] = [NSValue valueWithPointer:recognizer];
    }

    const SherpaOnnxOfflineStream *stream = SherpaOnnxCreateOfflineStream(recognizer);
    if (stream == nil) {
      reject(@"transcribe_error", @"端侧转写失败，请重试", nil);
      return;
    }
    SherpaOnnxAcceptWaveformOffline(stream, 16000, samples.data(), (int32_t)samples.size());
    SherpaOnnxDecodeOfflineStream(recognizer, stream);
    const SherpaOnnxOfflineRecognizerResult *result = SherpaOnnxGetOfflineStreamResult(stream);
    double duration = (double)samples.size() / 16000.0;
    NSMutableArray<NSDictionary *> *segments = [RNMacSenseVoice cuesFromResult:result duration:duration];
    SherpaOnnxDestroyOfflineRecognizerResult(result);
    SherpaOnnxDestroyOfflineStream(stream);
    resolve(@{ @"segments": segments, @"duration": @(duration) });
  });
}

@end

// ── RNMacPdf：PDF 阅读（PDFKit 渲染 + 目录树）────────────────────────────
// 与 Windows 的 RNWindowsPdf 共享同一接口：open 返回页面尺寸与目录树，
// renderPage 把整页渲染成 PNG base64，由共享 JS 阅读器排版。

#import <PDFKit/PDFKit.h>

@interface RNMacPdf : NSObject <RCTBridgeModule>
@end

@implementation RNMacPdf

RCT_EXPORT_MODULE(RNMacPdf);

+ (BOOL)requiresMainQueueSetup { return NO; }

static NSMutableDictionary<NSString *, PDFDocument *> *PdfDocuments(void) {
  static NSMutableDictionary<NSString *, PDFDocument *> *documents = nil;
  static dispatch_once_t once;
  dispatch_once(&once, ^{ documents = [NSMutableDictionary dictionary]; });
  return documents;
}

- (dispatch_queue_t)pdfQueue {
  static dispatch_queue_t queue = nil;
  static dispatch_once_t once;
  dispatch_once(&once, ^{ queue = dispatch_queue_create("app.clarora.pdf", DISPATCH_QUEUE_SERIAL); });
  return queue;
}

// 递归收集目录树；total 为节点上限，防止异常 PDF 撑爆桥接。
- (void)collectOutline:(PDFOutline *)outline
                  into:(NSMutableArray<NSDictionary *> *)out
                 depth:(NSInteger)depth
                remaining:(NSInteger *)remaining {
  if (outline == nil || *remaining <= 0 || depth > 8) return;
  NSInteger count = outline.numberOfChildren;
  for (NSInteger i = 0; i < count && *remaining > 0; i++) {
    PDFOutline *child = [outline childAtIndex:i];
    if (child == nil) continue;
    (*remaining)--;
    NSInteger page = -1;
    PDFDestination *destination = child.destination;
    if (destination != nil && destination.page != nil) {
      page = [destination.page.document indexForPage:destination.page];
    }
    NSMutableArray<NSDictionary *> *children = [NSMutableArray array];
    [self collectOutline:child into:children depth:depth + 1 remaining:remaining];
    [out addObject:@{ @"title": child.label ?: @"", @"page": @(page), @"children": children }];
  }
}

RCT_EXPORT_METHOD(open:(NSString *)path
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(self.pdfQueue, ^{
    PDFDocument *document = PdfDocuments()[path];
    if (document == nil) {
      document = [[PDFDocument alloc] initWithURL:[NSURL fileURLWithPath:path]];
      if (document == nil || document.isLocked || document.pageCount == 0) {
        reject(@"pdf_error", @"无法读取 PDF 文件（可能已加密或损坏）", nil);
        return;
      }
      PdfDocuments()[path] = document;
    }

    NSMutableArray<NSDictionary *> *pages = [NSMutableArray array];
    for (NSInteger i = 0; i < document.pageCount; i++) {
      CGRect bounds = [[document pageAtIndex:i] boundsForBox:kPDFDisplayBoxMediaBox];
      [pages addObject:@{ @"width": @(bounds.size.width), @"height": @(bounds.size.height) }];
    }

    NSMutableArray<NSDictionary *> *outline = [NSMutableArray array];
    NSInteger remaining = 500;
    [self collectOutline:document.outlineRoot into:outline depth:0 remaining:&remaining];
    resolve(@{ @"pages": pages, @"outline": outline });
  });
}

RCT_EXPORT_METHOD(renderPage:(NSString *)path
                  index:(NSInteger)index
                  width:(double)widthPx
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(self.pdfQueue, ^{
    PDFDocument *document = PdfDocuments()[path];
    if (document == nil || index < 0 || index >= document.pageCount) {
      reject(@"pdf_error", @"请先打开 PDF 文件", nil);
      return;
    }
    PDFPage *page = [document pageAtIndex:index];
    CGRect bounds = [page boundsForBox:kPDFDisplayBoxMediaBox];
    if (bounds.size.width <= 0) { reject(@"pdf_error", @"页面尺寸异常", nil); return; }
    double scale = widthPx / bounds.size.width;
    CGSize target = CGSizeMake(round(widthPx), round(bounds.size.height * scale));

    // PDFKit 的 thumbnailOfSize 在部分文档上拿不到 CGImage；位图上下文 +
    // drawWithBox 是稳定的整页渲染路径（白底，PDF 坐标系翻转后绘制）。
    CGColorSpaceRef space = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(NULL, target.width, target.height, 8,
                                                 target.width * 4, space,
                                                 kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
    CGColorSpaceRelease(space);
    if (context == nil) { reject(@"pdf_error", @"页面渲染失败", nil); return; }
    CGContextSetRGBFillColor(context, 1, 1, 1, 1);
    CGContextFillRect(context, CGRectMake(0, 0, target.width, target.height));
    // drawWithBox 自带坐标系翻转，这里只做点→像素缩放。
    CGContextScaleCTM(context, scale, scale);
    [page drawWithBox:kPDFDisplayBoxMediaBox toContext:context];
    CGImageRef rendered = CGBitmapContextCreateImage(context);
    CGContextRelease(context);
    if (rendered == nil) { reject(@"pdf_error", @"页面渲染失败", nil); return; }
    NSBitmapImageRep *rep = [[NSBitmapImageRep alloc] initWithCGImage:rendered];
    CGImageRelease(rendered);
    NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
    resolve(@{ @"png": [png base64EncodedStringWithOptions:NSDataBase64Encoding64CharacterLineLength] });
  });
}

RCT_EXPORT_METHOD(close:(NSString *)path
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  dispatch_async(self.pdfQueue, ^{
    [PdfDocuments() removeObjectForKey:path];
    resolve(@{ @"closed": @YES });
  });
}

@end

// ── RNMacKeychain：系统钥匙串存取（AI / 存储密钥等敏感配置）────────────────
// 通用密码项：service 固定为 app.clarora.secrets，account 区分用途。

#import <Security/Security.h>

@interface RNMacKeychain : NSObject <RCTBridgeModule>
@end

@implementation RNMacKeychain

RCT_EXPORT_MODULE(RNMacKeychain);

static NSDictionary *KeychainQuery(NSString *account) {
  return @{
    (id)kSecClass: (id)kSecClassGenericPassword,
    (id)kSecAttrService: @"app.clarora.secrets",
    (id)kSecAttrAccount: account,
  };
}

RCT_EXPORT_METHOD(setSecret:(NSString *)account
                  value:(NSString *)value
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSData *data = [value dataUsingEncoding:NSUTF8StringEncoding] ?: [NSData data];
  NSDictionary *query = KeychainQuery(account);
  // 先更新既有条目：重签名后旧条目仍挂在旧二进制的 ACL 上，更新会触发
  // 授权框并允许"始终允许"把新二进制加入信任列表；直接删旧建新则会让
  // 删除被拒后 SecItemAdd 撞 errSecDuplicateItem，写入从此全部失败。
  OSStatus status = SecItemUpdate((__bridge CFDictionaryRef)query, (__bridge CFDictionaryRef)@{
    (__bridge NSString *)kSecValueData: data,
    (__bridge NSString *)kSecAttrAccessible: (id)kSecAttrAccessibleAfterFirstUnlock,
  });
  if (status == errSecItemNotFound) {
    NSMutableDictionary *attributes = [query mutableCopy];
    attributes[(__bridge NSString *)kSecValueData] = data;
    attributes[(__bridge NSString *)kSecAttrAccessible] = (id)kSecAttrAccessibleAfterFirstUnlock;
    status = SecItemAdd((__bridge CFDictionaryRef)attributes, NULL);
  }
  if (status == errSecSuccess) resolve(@YES);
  else reject(@"keychain_error", [NSString stringWithFormat:@"钥匙串写入失败（OSStatus %d）", (int)status], nil);
}

RCT_EXPORT_METHOD(getSecret:(NSString *)account
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  NSMutableDictionary *query = [KeychainQuery(account) mutableCopy];
  query[(__bridge NSString *)kSecReturnData] = @YES;
  query[(__bridge NSString *)kSecMatchLimit] = (id)kSecMatchLimitOne;
  CFDataRef data = NULL;
  OSStatus status = SecItemCopyMatching((__bridge CFDictionaryRef)query, (CFTypeRef *)&data);
  if (status == errSecItemNotFound) { resolve([NSNull null]); return; }
  if (status != errSecSuccess) {
    reject(@"keychain_error", [NSString stringWithFormat:@"钥匙串读取失败（OSStatus %d）", (int)status], nil);
    return;
  }
  NSString *value = [[NSString alloc] initWithData:(__bridge NSData *)data encoding:NSUTF8StringEncoding];
  CFRelease(data);
  resolve(value ?: @"");
}

RCT_EXPORT_METHOD(deleteSecret:(NSString *)account
                  resolver:(RCTPromiseResolveBlock)resolve
                  rejecter:(RCTPromiseRejectBlock)reject)
{
  OSStatus status = SecItemDelete((__bridge CFDictionaryRef)KeychainQuery(account));
  if (status == errSecSuccess || status == errSecItemNotFound) resolve(@YES);
  else reject(@"keychain_error", [NSString stringWithFormat:@"钥匙串删除失败（OSStatus %d）", (int)status], nil);
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
