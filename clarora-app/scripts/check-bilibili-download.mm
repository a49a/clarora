// Native integration driver. Builds the same downloader as the app, without React.
#define CLARORA_DOWNLOAD_STANDALONE 1
#import "../macos/Clarora-macOS/CLBilibiliDownloader.mm"

int main(int argc, const char *argv[]) {
  @autoreleasepool {
    if (argc < 2) return 2;
    CLBilibiliDownloader *downloader = [CLBilibiliDownloader new];
    __block BOOL done = NO;
    __block int status = 1;
    [downloader downloadURL:[NSString stringWithUTF8String:argv[1]] completion:^(NSString *path, NSString *error) {
      NSDictionary *result = path ? @{@"path": path} : @{@"error": error ?: @"unknown"};
      NSData *json = [NSJSONSerialization dataWithJSONObject:result options:0 error:nil];
      puts([[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding].UTF8String);
      status = path ? 0 : 1;
      done = YES;
    }];
    if (argc > 2) dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 200 * NSEC_PER_MSEC), dispatch_get_main_queue(), ^{ [downloader cancel]; });
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:1900];
    while (!done && deadline.timeIntervalSinceNow > 0) {
      [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
    }
    return done ? status : 3;
  }
}
