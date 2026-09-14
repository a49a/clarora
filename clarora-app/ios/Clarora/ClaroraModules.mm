#import <UIKit/UIKit.h>
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <React/RCTBridgeModule.h>
#import <React/RCTUtils.h>

@interface RNIOSFilePicker : NSObject <RCTBridgeModule, UIDocumentPickerDelegate>
@property(nonatomic, copy) RCTPromiseResolveBlock resolve;
@property(nonatomic, copy) RCTPromiseRejectBlock reject;
@end

@implementation RNIOSFilePicker
RCT_EXPORT_MODULE();
+ (BOOL)requiresMainQueueSetup { return YES; }
- (dispatch_queue_t)methodQueue { return dispatch_get_main_queue(); }
RCT_EXPORT_METHOD(pickFile:(NSDictionary *)options resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  if (self.resolve) { reject(@"picker_busy", @"请先完成当前文件选择", nil); return; }
  UIViewController *presenter = RCTPresentedViewController();
  if (!presenter) { reject(@"picker_unavailable", @"无法打开文件选择器", nil); return; }
  NSString *mime = options[@"type"];
  UTType *type = UTTypeData;
  if ([mime hasPrefix:@"audio/"]) type = UTTypeAudio;
  else if ([mime hasPrefix:@"image/"]) type = UTTypeImage;
  else if (mime.length && ![mime isEqualToString:@"*/*"]) type = [UTType typeWithMIMEType:mime] ?: UTTypeData;
  self.resolve = resolve;
  self.reject = reject;
  // Import a copy: URLs from iCloud/other providers must not outlive their access grant.
  UIDocumentPickerViewController *picker = [[UIDocumentPickerViewController alloc] initForOpeningContentTypes:@[type] asCopy:YES];
  picker.delegate = self;
  picker.allowsMultipleSelection = NO;
  [presenter presentViewController:picker animated:YES completion:nil];
}
- (void)documentPicker:(UIDocumentPickerViewController *)controller didPickDocumentsAtURLs:(NSArray<NSURL *> *)urls
{
  NSURL *source = urls.firstObject;
  if (!source) { [self documentPickerWasCancelled:controller]; return; }
  BOOL access = [source startAccessingSecurityScopedResource];
  NSURL *directory = [[[NSFileManager defaultManager] URLsForDirectory:NSCachesDirectory inDomains:NSUserDomainMask].firstObject URLByAppendingPathComponent:NSUUID.UUID.UUIDString isDirectory:YES];
  NSError *error = nil;
  BOOL copied = [[NSFileManager defaultManager] createDirectoryAtURL:directory withIntermediateDirectories:YES attributes:nil error:&error];
  NSURL *destination = [directory URLByAppendingPathComponent:source.lastPathComponent];
  if (copied) copied = [[NSFileManager defaultManager] copyItemAtURL:source toURL:destination error:&error];
  if (access) [source stopAccessingSecurityScopedResource];
  if (copied) self.resolve(@{@"uri": destination.absoluteString, @"name": source.lastPathComponent});
  else self.reject(@"file_import_failed", error.localizedDescription ?: @"文件导入失败", error);
  self.resolve = nil; self.reject = nil;
}
- (void)documentPickerWasCancelled:(UIDocumentPickerViewController *)controller
{
  if (self.resolve) self.resolve(nil);
  self.resolve = nil; self.reject = nil;
}
@end

@interface RNIOSClipboard : NSObject <RCTBridgeModule>
@end
@implementation RNIOSClipboard
RCT_EXPORT_MODULE();
+ (BOOL)requiresMainQueueSetup { return NO; }
RCT_EXPORT_METHOD(setText:(NSString *)text)
{
  dispatch_async(dispatch_get_main_queue(), ^{ UIPasteboard.generalPasteboard.string = text ?: @""; });
}
@end

@interface RNIOSAudio : NSObject <RCTBridgeModule, AVAudioPlayerDelegate>
@property(nonatomic, strong) AVAudioPlayer *player;
@property(nonatomic, strong) AVAudioPlayer *feed0;
@property(nonatomic, strong) AVAudioPlayer *feed1;
@property(nonatomic, strong) AVAudioPlayer *breakPlayer;
@property(nonatomic, strong) AVAudioRecorder *recorder;
@property(nonatomic) BOOL completed;
@end

@implementation RNIOSAudio
RCT_EXPORT_MODULE();
+ (BOOL)requiresMainQueueSetup { return NO; }
- (dispatch_queue_t)methodQueue { return dispatch_get_main_queue(); }
- (instancetype)init
{
  if ((self = [super init])) {
    [[NSNotificationCenter defaultCenter] addObserver:self selector:@selector(interrupted:) name:AVAudioSessionInterruptionNotification object:nil];
  }
  return self;
}
- (void)dealloc { [[NSNotificationCenter defaultCenter] removeObserver:self]; }
- (void)interrupted:(NSNotification *)notification
{
  if ([notification.userInfo[AVAudioSessionInterruptionTypeKey] unsignedIntegerValue] == AVAudioSessionInterruptionTypeBegan) {
    [self.player pause]; [self.feed0 pause]; [self.feed1 pause]; [self.breakPlayer pause];
    [self.recorder stop];
  }
}
- (BOOL)activate:(BOOL)record error:(NSError **)error
{
  AVAudioSession *session = AVAudioSession.sharedInstance;
  return [session setCategory:record ? AVAudioSessionCategoryPlayAndRecord : AVAudioSessionCategoryPlayback
                      mode:AVAudioSessionModeDefault
                   options:record ? AVAudioSessionCategoryOptionDefaultToSpeaker : 0 error:error]
      && [session setActive:YES error:error];
}
- (AVAudioPlayer *)open:(NSString *)path error:(NSError **)error
{
  NSURL *url = [path hasPrefix:@"file://"] ? [NSURL URLWithString:path] : [NSURL fileURLWithPath:path];
  AVAudioPlayer *player = [[AVAudioPlayer alloc] initWithContentsOfURL:url error:error];
  player.enableRate = YES;
  [player prepareToPlay];
  return player;
}
- (NSDictionary *)snapshot:(AVAudioPlayer *)player
{
  if (!player) return @{@"isLoaded": @NO, @"isPlaying": @NO, @"positionMillis": @0};
  return @{@"isLoaded": @YES, @"isPlaying": @(player.isPlaying), @"positionMillis": @(player.currentTime * 1000),
           @"durationMillis": @(player.duration * 1000), @"didJustFinish": @(player == self.player && self.completed)};
}
- (void)audioPlayerDidFinishPlaying:(AVAudioPlayer *)player successfully:(BOOL)flag
{
  if (player == self.player) self.completed = flag;
}
RCT_EXPORT_METHOD(load:(NSString *)path rate:(double)rate resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  [self.player stop]; self.player = nil; self.completed = NO;
  NSError *error = nil;
  self.player = [self open:path error:&error];
  if (!self.player) { reject(@"audio_load", error.localizedDescription, error); return; }
  self.player.delegate = self;
  self.player.rate = rate;
  resolve([self snapshot:self.player]);
}
RCT_EXPORT_METHOD(play:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  NSError *error = nil;
  if (!self.player || ![self activate:NO error:&error] || ![self.player play]) {
    reject(@"audio_play", error.localizedDescription ?: @"无法播放音频", error); return;
  }
  self.completed = NO; resolve([self snapshot:self.player]);
}
RCT_EXPORT_METHOD(pause:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{ [self.player pause]; resolve([self snapshot:self.player]); }
RCT_EXPORT_METHOD(status:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{ resolve([self snapshot:self.player]); }
RCT_EXPORT_METHOD(setPosition:(double)ms resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{ self.player.currentTime = MAX(0, MIN(ms / 1000, self.player.duration)); self.completed = NO; resolve([self snapshot:self.player]); }
RCT_EXPORT_METHOD(setRate:(double)rate resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{ self.player.rate = rate; resolve([self snapshot:self.player]); }
RCT_EXPORT_METHOD(setLoop:(BOOL)loop resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{ self.player.numberOfLoops = loop ? -1 : 0; resolve([self snapshot:self.player]); }
RCT_EXPORT_METHOD(unload:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{ [self.player stop]; self.player = nil; self.completed = NO; resolve(nil); }
- (AVAudioPlayer *)slot:(NSInteger)slot { return slot == 0 ? self.feed0 : self.feed1; }
RCT_EXPORT_METHOD(feedPrepare:(NSInteger)slot path:(NSString *)path startMs:(double)ms resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  if (slot < 0 || slot > 1) { reject(@"feed_slot", @"无效的预加载位置", nil); return; }
  [[self slot:slot] stop];
  NSError *error = nil;
  AVAudioPlayer *player = [self open:path error:&error];
  if (slot == 0) self.feed0 = player; else self.feed1 = player;
  if (!player) { reject(@"feed_load", error.localizedDescription, error); return; }
  player.currentTime = MAX(0, MIN(ms / 1000, player.duration));
  resolve([self snapshot:player]);
}
RCT_EXPORT_METHOD(feedPlay:(NSInteger)slot resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  NSError *error = nil;
  AVAudioPlayer *player = [self slot:slot];
  if (!player || ![self activate:NO error:&error] || ![player play]) { reject(@"feed_play", error.localizedDescription ?: @"片段播放失败", error); return; }
  resolve([self snapshot:player]);
}
RCT_EXPORT_METHOD(feedPause:(NSInteger)slot resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{ AVAudioPlayer *player = [self slot:slot]; [player pause]; resolve([self snapshot:player]); }
RCT_EXPORT_METHOD(feedStatus:(NSInteger)slot resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{ resolve([self snapshot:[self slot:slot]]); }
RCT_EXPORT_METHOD(feedUnload:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{ [self.feed0 stop]; [self.feed1 stop]; self.feed0 = nil; self.feed1 = nil; resolve(nil); }
RCT_EXPORT_METHOD(startRecording:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  [AVAudioSession.sharedInstance requestRecordPermission:^(BOOL granted) {
    dispatch_async(dispatch_get_main_queue(), ^{
      if (!granted) { reject(@"microphone_denied", @"请在系统设置中允许麦克风权限", nil); return; }
      if (self.recorder) { reject(@"recording_busy", @"请先结束当前录音", nil); return; }
      [self.player pause]; [self.feed0 pause]; [self.feed1 pause]; [self.breakPlayer pause];
      NSError *error = nil;
      if (![self activate:YES error:&error]) { reject(@"record_session", error.localizedDescription, error); return; }
      NSURL *url = [NSURL fileURLWithPath:[NSTemporaryDirectory() stringByAppendingPathComponent:[NSUUID.UUID.UUIDString stringByAppendingString:@".m4a"]]];
      self.recorder = [[AVAudioRecorder alloc] initWithURL:url settings:@{AVFormatIDKey: @(kAudioFormatMPEG4AAC), AVSampleRateKey: @44100, AVNumberOfChannelsKey: @1, AVEncoderAudioQualityKey: @(AVAudioQualityHigh)} error:&error];
      if (!self.recorder || ![self.recorder record]) {
        self.recorder = nil;
        [self activate:NO error:nil];
        reject(@"record_start", error.localizedDescription ?: @"录音启动失败", error); return;
      }
      resolve(@YES);
    });
  }];
}
RCT_EXPORT_METHOD(stopRecording:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject)
{
  if (!self.recorder) { reject(@"record_missing", @"没有进行中的录音", nil); return; }
  NSURL *url = self.recorder.url;
  [self.recorder stop]; self.recorder = nil;
  [self activate:NO error:nil];
  resolve(@{@"uri": url.absoluteString, @"name": url.lastPathComponent});
}
RCT_EXPORT_METHOD(playChime) { AudioServicesPlaySystemSound(1057); }
RCT_EXPORT_METHOD(playBreak:(NSString *)path)
{
  [self.breakPlayer stop];
  if (![self activate:NO error:nil]) return;
  self.breakPlayer = [self open:path error:nil]; self.breakPlayer.numberOfLoops = -1;
  [self.breakPlayer play];
}
RCT_EXPORT_METHOD(pauseBreak) { [self.breakPlayer pause]; }
RCT_EXPORT_METHOD(resumeBreak) { if ([self activate:NO error:nil]) [self.breakPlayer play]; }
RCT_EXPORT_METHOD(stopBreak) { [self.breakPlayer stop]; self.breakPlayer = nil; }
@end
