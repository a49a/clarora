#import <Foundation/Foundation.h>
#import <spawn.h>
#import <signal.h>
#import <sys/wait.h>
#import <unistd.h>
extern char **environ;

// Each download owns a directory and a process group (including ffmpeg).
@interface CLBilibiliDownloader : NSObject
@property(nonatomic) BOOL busy;
@property(nonatomic) BOOL cancelled;
@property(nonatomic) pid_t processID;
- (void)downloadURL:(NSString *)url completion:(void (^)(NSString *, NSString *))completion;
- (void)cancel;
@end

@implementation CLBilibiliDownloader
- (NSString *)findTool:(NSString *)name {
  NSFileManager *fm = NSFileManager.defaultManager;
  NSString *search = [NSString stringWithFormat:@"%@:/opt/homebrew/bin:/usr/local/bin:/usr/bin:%@/.local/bin",
                      NSProcessInfo.processInfo.environment[@"PATH"] ?: @"", NSHomeDirectory()];
  for (NSString *directory in [search componentsSeparatedByString:@":"]) {
    if (![directory hasPrefix:@"/"]) continue;
    NSString *path = [directory stringByAppendingPathComponent:name];
    if ([fm isExecutableFileAtPath:path]) return path;
  }
  return nil;
}

- (void)cancel {
  @synchronized(self) {
    if (!self.busy) return;
    self.cancelled = YES;
    // SIGKILL ensures child writers stop before the temporary directory is removed.
    if (self.processID > 0) kill(-self.processID, SIGKILL);
  }
}

- (void)downloadURL:(NSString *)url completion:(void (^)(NSString *, NSString *))completion {
  @synchronized(self) {
    if (self.busy) { completion(nil, @"已有下载任务，请等待或取消后重试"); return; }
    self.busy = YES;
    self.cancelled = NO;
  }
  dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
    NSFileManager *fm = NSFileManager.defaultManager;
    NSString *directory = nil;
    NSString *result = nil;
    NSString *failure = nil;
    @try {
      // Validate again at the native boundary. Only normalized URLs reach the tool.
      NSRegularExpression *pattern = [NSRegularExpression regularExpressionWithPattern:
        @"^https://www\\.bilibili\\.com/video/(BV[0-9A-Za-z]{10}|av[1-9][0-9]*)/?(\\?p=[1-9][0-9]{0,4})?$" options:0 error:nil];
      if ([pattern numberOfMatchesInString:url options:0 range:NSMakeRange(0, url.length)] != 1)
        @throw [NSException exceptionWithName:@"input" reason:@"请输入 bilibili 视频链接或 BV 号" userInfo:nil];
      NSString *tool = [self findTool:@"yt-dlp"];
      NSString *ffmpeg = [self findTool:@"ffmpeg"];
      if (!tool || !ffmpeg)
        @throw [NSException exceptionWithName:@"tools" reason:@"需要安装 yt-dlp 和 ffmpeg：在终端执行 brew install yt-dlp ffmpeg，然后重试。" userInfo:nil];
      NSString *documents = [fm URLsForDirectory:NSApplicationSupportDirectory inDomains:NSUserDomainMask].firstObject.path;
      directory = [[documents stringByAppendingPathComponent:@"Clarora/bilibili"] stringByAppendingPathComponent:NSUUID.UUID.UUIDString];
      NSError *mkdirError = nil;
      if (![fm createDirectoryAtPath:directory withIntermediateDirectories:YES attributes:nil error:&mkdirError])
        @throw [NSException exceptionWithName:@"directory" reason:mkdirError.localizedDescription userInfo:nil];
      NSArray<NSString *> *arguments = @[
        tool, @"--ignore-config", @"--no-playlist", @"--no-simulate", @"--no-progress", @"--no-color",
        @"--socket-timeout", @"30", @"--retries", @"2", @"--fragment-retries", @"2",
        @"--ffmpeg-location", [ffmpeg stringByDeletingLastPathComponent],
        @"--format", @"bv*[vcodec^=avc1]+ba[acodec^=mp4a]/b[ext=mp4]", @"--merge-output-format", @"mp4", @"--remux-video", @"mp4",
        @"--write-subs", @"--sub-langs", @"en.*,zh.*,ai-en.*,ai-zh.*", @"--embed-subs",
        @"--print", @"after_move:%(filepath)j", @"--paths", directory,
        @"--output", @"%(title).100B [%(id)s].%(ext)s", @"--", url
      ];
      NSPipe *out = [NSPipe pipe], *err = [NSPipe pipe];
      posix_spawn_file_actions_t actions;
      posix_spawn_file_actions_init(&actions);
      posix_spawn_file_actions_adddup2(&actions, out.fileHandleForWriting.fileDescriptor, STDOUT_FILENO);
      posix_spawn_file_actions_adddup2(&actions, err.fileHandleForWriting.fileDescriptor, STDERR_FILENO);
      posix_spawnattr_t attributes;
      posix_spawnattr_init(&attributes);
      posix_spawnattr_setflags(&attributes, POSIX_SPAWN_SETPGROUP | POSIX_SPAWN_CLOEXEC_DEFAULT);
      posix_spawnattr_setpgroup(&attributes, 0);
      char **argv = (char **)calloc(arguments.count + 1, sizeof(char *));
      for (NSUInteger i = 0; i < arguments.count; i++) argv[i] = strdup(arguments[i].UTF8String);
      pid_t pid = 0;
      int launchError;
      @synchronized(self) {
        launchError = self.cancelled ? ECANCELED : posix_spawn(&pid, tool.fileSystemRepresentation, &actions, &attributes, argv, environ);
        if (launchError == 0) self.processID = pid;
      }
      for (NSUInteger i = 0; i < arguments.count; i++) free(argv[i]);
      free(argv);
      posix_spawn_file_actions_destroy(&actions);
      posix_spawnattr_destroy(&attributes);
      [out.fileHandleForWriting closeFile];
      [err.fileHandleForWriting closeFile];
      if (launchError != 0)
        @throw [NSException exceptionWithName:@"launch" reason:[NSString stringWithFormat:@"无法启动下载器：%s", strerror(launchError)] userInfo:nil];

      // Drain both streams concurrently, retaining only bounded tails.
      dispatch_group_t readers = dispatch_group_create();
      NSMutableData *stdoutData = [NSMutableData data], *stderrData = [NSMutableData data];
      NSArray *pipes = @[out, err], *buffers = @[stdoutData, stderrData];
      for (NSUInteger i = 0; i < pipes.count; i++) {
        NSPipe *pipe = pipes[i]; NSMutableData *buffer = buffers[i];
        dispatch_group_async(readers, dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
          while (YES) {
            NSData *chunk = [pipe.fileHandleForReading readDataOfLength:8192];
            if (chunk.length == 0) break;
            [buffer appendData:chunk];
            if (buffer.length > 65536) [buffer replaceBytesInRange:NSMakeRange(0, buffer.length - 65536) withBytes:NULL length:0];
          }
          [pipe.fileHandleForReading closeFile];
        });
      }
      // All waits remain on the worker queue. Timeout and cancellation kill the group.
      __block BOOL timedOut = NO;
      dispatch_source_t timer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, dispatch_get_global_queue(QOS_CLASS_UTILITY, 0));
      dispatch_source_set_timer(timer, dispatch_time(DISPATCH_TIME_NOW, 1800LL * NSEC_PER_SEC), DISPATCH_TIME_FOREVER, NSEC_PER_SEC);
      dispatch_source_set_event_handler(timer, ^{
        @synchronized(self) { if (self.processID == pid) { timedOut = YES; kill(-pid, SIGKILL); } }
      });
      dispatch_resume(timer);
      int status = 0;
      pid_t waited;
      do { waited = waitpid(pid, &status, 0); } while (waited < 0 && errno == EINTR);
      @synchronized(self) { self.processID = 0; }
      dispatch_source_cancel(timer);
      // Also stop any orphan child holding a pipe open after an abnormal parent exit.
      kill(-pid, SIGKILL);
      dispatch_group_wait(readers, DISPATCH_TIME_FOREVER);
      NSString *stderrText = [[NSString alloc] initWithData:stderrData encoding:NSUTF8StringEncoding] ?: @"";
      if (timedOut) failure = @"下载超过 30 分钟，已停止。请检查网络后重试。";
      else if (waited < 0 || !WIFEXITED(status) || WEXITSTATUS(status) != 0)
        failure = [NSString stringWithFormat:@"下载失败，请检查网络、视频访问权限，或更新 yt-dlp 后重试。\n%@", [stderrText substringFromIndex:stderrText.length > 2000 ? stderrText.length - 2000 : 0]];
      else {
        NSString *output = [[NSString alloc] initWithData:stdoutData encoding:NSUTF8StringEncoding] ?: @"";
        for (NSString *line in [output componentsSeparatedByCharactersInSet:NSCharacterSet.newlineCharacterSet]) {
          id value = [NSJSONSerialization JSONObjectWithData:[line dataUsingEncoding:NSUTF8StringEncoding] options:NSJSONReadingFragmentsAllowed error:nil];
          if (![value isKindOfClass:NSString.class]) continue;
          NSString *path = [value stringByResolvingSymlinksInPath];
          NSString *prefix = [[directory stringByResolvingSymlinksInPath] stringByAppendingString:@"/"];
          NSDictionary *info = [fm attributesOfItemAtPath:path error:nil];
          if ([path hasPrefix:prefix] && [path.pathExtension.lowercaseString isEqualToString:@"mp4"] &&
              [info[NSFileType] isEqual:NSFileTypeRegular] && [info[NSFileSize] unsignedLongLongValue] > 0) result = path;
        }
        if (!result) failure = @"下载器没有返回完整视频文件，请更新 yt-dlp 后重试。";
      }
    } @catch (NSException *exception) { failure = exception.reason ?: @"下载失败"; }
    @synchronized(self) {
      if (self.cancelled) { failure = @"已取消下载"; result = nil; }
      if (!result && directory) [fm removeItemAtPath:directory error:nil];
      self.processID = 0;
      self.busy = NO;
    }
    dispatch_async(dispatch_get_main_queue(), ^{ completion(result, failure); });
  });
}
@end

#ifndef CLARORA_DOWNLOAD_STANDALONE
#import <React/RCTBridgeModule.h>
@interface RNBilibiliDownloader : NSObject <RCTBridgeModule>
@property(nonatomic, strong) CLBilibiliDownloader *downloader;
@end
@implementation RNBilibiliDownloader
RCT_EXPORT_MODULE();
- (instancetype)init { if ((self = [super init])) _downloader = [CLBilibiliDownloader new]; return self; }
+ (BOOL)requiresMainQueueSetup { return NO; }
RCT_EXPORT_METHOD(download:(NSString *)url resolver:(RCTPromiseResolveBlock)resolve rejecter:(RCTPromiseRejectBlock)reject) {
  [self.downloader downloadURL:url completion:^(NSString *path, NSString *error) {
    if (path) resolve(path); else reject(@"bilibili_download", error, nil);
  }];
}
RCT_EXPORT_METHOD(cancel) { [self.downloader cancel]; }
- (void)invalidate { [self.downloader cancel]; }
@end
#endif
