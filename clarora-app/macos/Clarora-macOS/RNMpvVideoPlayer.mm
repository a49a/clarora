// ── libmpv video player (English video learning, Mac only) ─────────────────
// Same engine IINA uses: mpv (which embeds FFmpeg) demuxes and decodes, so
// any container plays — MKV, AVI, WebM, with DD+/DTS/Opus audio — instead of
// only the AVFoundation-blessed mp4/mov family. The split of responsibilities
// mirrors IINA too: mpv owns decoding and its own event thread, a CAOpenGLLayer
// paints frames through the render API, and the JS transport UI stays in
// charge — src/playing/rate go down as props, onProgress/onEnd/onError come
// back as events, seeks go through RNVideoControl.

#import <AppKit/AppKit.h>
#import <OpenGL/gl3.h>
#import <React/RCTBridgeModule.h>
#import <React/RCTComponent.h>
#import <React/RCTUIManager.h>
#import <React/RCTUIKit.h>
#import <React/RCTViewManager.h>

#import <mpv/client.h>
#import <mpv/render.h>
#import <mpv/render_gl.h>

// CAOpenGLLayer and the CGL pixel-format calls below are deprecated in favor
// of Metal, but the libmpv render API only consumes OpenGL, so every libmpv
// macOS client (IINA included) still goes through them.
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

static void *RNMpvGLGetProcAddress(void *ctx, const char *name)
{
  // Resolve OpenGL entry points from the framework bundle: unlike
  // CGLGetCurrentContext-based lookups this works before any context is
  // current, which mpv_render_context_create relies on.
  CFBundleRef glBundle = CFBundleGetBundleWithIdentifier(CFSTR("com.apple.opengl"));
  return CFBundleGetFunctionPointerForName(glBundle, (__bridge CFStringRef)[NSString stringWithUTF8String:name]);
}

#pragma mark - GL layer

@interface RNMpvGLLayer : CAOpenGLLayer
@property (nonatomic, assign) mpv_handle *mpv;
@property (nonatomic, assign) mpv_render_context *renderContext;
@property (nonatomic, weak) id onReadyTarget;
@property (nonatomic, assign) SEL onReadySelector;
@end

@implementation RNMpvGLLayer

- (instancetype)init
{
  if (self = [super init]) {
    // Continuous redraw on Core Animation's own thread; canDraw gates it on
    // mpv actually having a new frame, so pauses cost nothing.
    self.asynchronous = YES;
    self.backgroundColor = NSColor.blackColor.CGColor;
  }
  return self;
}

- (CGLPixelFormatObj)copyCGLPixelFormatForDisplayMask:(uint32_t)mask
{
  const CGLPixelFormatAttribute coreProfileAttrs[] = {
    kCGLPFAOpenGLProfile, (CGLPixelFormatAttribute)kCGLOGLPVersion_3_2_Core,
    kCGLPFAAllowOfflineRenderers,
    (CGLPixelFormatAttribute)0
  };
  CGLPixelFormatObj format = NULL;
  GLint formatsFound = 0;
  if (CGLChoosePixelFormat(coreProfileAttrs, &format, &formatsFound) == kCGLNoError && format != NULL) {
    return format;
  }
  return [super copyCGLPixelFormatForDisplayMask:mask];
}

- (BOOL)canDrawInCGLContext:(CGLContextObj)glContext
                pixelFormat:(CGLPixelFormatObj)pixelFormat
               forLayerTime:(CFTimeInterval)timeInterval
                displayTime:(const CVTimeStamp *)timestamp
{
  if (self.mpv == NULL) return NO;
  // The first draw exists to bootstrap the render context below.
  if (self.renderContext == NULL) return YES;
  // Draining the update flags here is the contract: they say whether mpv has
  // queued a frame worth presenting since the last render.
  return (mpv_render_context_update(self.renderContext) & MPV_RENDER_UPDATE_FRAME) != 0;
}

- (void)drawInCGLContext:(CGLContextObj)glContext
             pixelFormat:(CGLPixelFormatObj)pixelFormat
            forLayerTime:(CFTimeInterval)timeInterval
             displayTime:(const CVTimeStamp *)timestamp
{
  // CAOpenGLLayer binds its own framebuffer for this callback and that one is
  // the only complete render target: the default framebuffer reads as
  // GL_FRAMEBUFFER_UNDEFINED under the GL-on-Metal renderer. mpv must render
  // into the id captured here, before mpv_render_context_create resets the
  // binding to 0 internally.
  GLint boundFbo = 0;
  glGetIntegerv(GL_FRAMEBUFFER_BINDING, &boundFbo);
  if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) return;

  // mpv needs a current GL context on the calling thread when the render
  // context is created (it calls glGetString during init — doing this with no
  // context current segfaults in libGL). CAOpenGLLayer only guarantees a
  // current context inside this draw callback, so creation is deferred here
  // and the layer's own context doubles as mpv's render target.
  if (self.renderContext == NULL && self.mpv != NULL) {
    mpv_opengl_init_params glInit = {
      .get_proc_address = RNMpvGLGetProcAddress,
      .get_proc_address_ctx = NULL,
    };
    mpv_render_param params[] = {
      { MPV_RENDER_PARAM_API_TYPE, (void *)MPV_RENDER_API_TYPE_OPENGL },
      { MPV_RENDER_PARAM_OPENGL_INIT_PARAMS, &glInit },
      { MPV_RENDER_PARAM_INVALID, NULL },
    };
    mpv_render_context *context = NULL;
    if (mpv_render_context_create(&context, self.mpv, params) >= 0) {
      self.renderContext = context;
      [self.onReadyTarget performSelector:self.onReadySelector];
    }
  }

  if (self.renderContext != NULL) {
    CGSize pixels = self.bounds.size;
    pixels.width *= self.contentsScale;
    pixels.height *= self.contentsScale;
    mpv_opengl_fbo fbo = { boundFbo, (int)pixels.width, (int)pixels.height, 0 };
    int flipY = 1;  // GL's origin is bottom-left, Core Animation's is top-left
    mpv_render_param params[] = {
      { MPV_RENDER_PARAM_OPENGL_FBO, &fbo },
      { MPV_RENDER_PARAM_FLIP_Y, &flipY },
      { MPV_RENDER_PARAM_INVALID, NULL },
    };
    mpv_render_context_render(self.renderContext, params);
    mpv_render_context_report_swap(self.renderContext);
  }
  // Deliberately no [super drawInCGLContext:]: the default implementation
  // paints background color over whatever was just rendered, which turns the
  // frame black. mpv fills letterbox bars itself.
}

@end

#pragma mark - Player view

@interface RNMpvPlayerView : RCTUIView
@property (nonatomic, copy) NSString *src;
@property (nonatomic, assign) BOOL playing;
@property (nonatomic, assign) CGFloat rate;
@property (nonatomic, copy) RCTDirectEventBlock onProgress;
@property (nonatomic, copy) RCTDirectEventBlock onEnd;
@property (nonatomic, copy) RCTDirectEventBlock onError;
@property (nonatomic, readonly) mpv_handle *mpvHandle;
@end

@implementation RNMpvPlayerView
{
  mpv_handle *_mpv;
  RNMpvGLLayer *_glLayer;
  NSThread *_eventThread;
  dispatch_semaphore_t _eventThreadDone;
  BOOL _eventThreadStopped;
  double _durationSec;
  double _lastReportedSec;
  NSString *_pendingLoadPath;
}

- (instancetype)initWithFrame:(NSRect)frame
{
  if (self = [super initWithFrame:frame]) {
    self.wantsLayer = YES;
    _glLayer = [RNMpvGLLayer new];
    _glLayer.frame = self.bounds;
    [self.layer addSublayer:_glLayer];

    _mpv = mpv_create();
    if (_mpv == NULL) return self;

    // vo=libmpv hands frame presentation to the render API instead of mpv
    // creating its own window, which is what lets it live inside an RN view.
    mpv_set_option_string(_mpv, "vo", "libmpv");
    mpv_set_option_string(_mpv, "hwdec", "auto-safe");
    mpv_set_option_string(_mpv, "terminal", "no");
    mpv_set_option_string(_mpv, "osc", "no");
    mpv_set_option_string(_mpv, "input-default-bindings", "no");
    mpv_set_option_string(_mpv, "input-vo-keyboard", "no");
    // MKV cover art otherwise attaches as a video track and blanks the screen.
    mpv_set_option_string(_mpv, "audio-display", "no");
    // Park on the last frame at EOF instead of going idle, so "seek 0 + play"
    // from JS can loop a clip; EOF is then reported via the eof-reached
    // property below (END_FILE does not fire in keep-open mode).
    mpv_set_option_string(_mpv, "keep-open", "always");
    // Load every subtitle file sitting next to the video (VobSub pairs and
    // srt whose names don't match the release-style video name), and prefer
    // English then Chinese when auto-selecting a track.
    mpv_set_option_string(_mpv, "sub-auto", "all");
    mpv_set_option_string(_mpv, "slang", "en,eng,zh,chi,chs");
    mpv_request_log_messages(_mpv, "error");
    if (mpv_initialize(_mpv) < 0) {
      mpv_destroy(_mpv);
      _mpv = NULL;
      return self;
    }
    // The render context itself is created by the layer on its first draw,
    // where a current GL context is guaranteed.
    _glLayer.onReadyTarget = self;
    _glLayer.onReadySelector = @selector(mpvRenderContextReady);
    _glLayer.mpv = _mpv;

    mpv_observe_property(_mpv, 0, "time-pos", MPV_FORMAT_DOUBLE);
    mpv_observe_property(_mpv, 0, "duration", MPV_FORMAT_DOUBLE);
    mpv_observe_property(_mpv, 0, "eof-reached", MPV_FORMAT_FLAG);

    _eventThreadDone = dispatch_semaphore_create(0);
    _eventThread = [[NSThread alloc] initWithTarget:self selector:@selector(eventLoop) object:nil];
    [_eventThread start];
  }
  return self;
}

- (void)layout
{
  [super layout];
  _glLayer.frame = self.bounds;
  _glLayer.contentsScale = self.window != nil ? self.window.backingScaleFactor : 2.0;
}

- (mpv_handle *)mpvHandle
{
  return _mpv;
}

// mpv has its own thread deliver property/end/log events here; the loop only
// reads playback state and hops to the main queue for the JS callbacks,
// never touching the GL layer.
- (void)eventLoop
{
  while (!_eventThreadStopped) {
    mpv_event *event = mpv_wait_event(_mpv, 0.5);
    if (event->event_id == MPV_EVENT_NONE || event->event_id == MPV_EVENT_TICK) continue;
    switch (event->event_id) {
      case MPV_EVENT_PROPERTY_CHANGE: {
        mpv_event_property *prop = (mpv_event_property *)event->data;
        if (prop->data == NULL) break;  // file unloaded or track not yet known
        if (strcmp(prop->name, "time-pos") == 0) {
          double sec = *(double *)prop->data;
          // Same 0.25 s cadence the JS progress bar expects; a seek backwards
          // always reports so the bar never stalls at an old position.
          if (fabs(sec - _lastReportedSec) >= 0.25 || sec < _lastReportedSec) {
            _lastReportedSec = sec;
            [self emitProgressAt:sec];
          }
        } else if (strcmp(prop->name, "duration") == 0) {
          _durationSec = *(double *)prop->data;
        } else if (strcmp(prop->name, "eof-reached") == 0) {
          // keep-open parks at the last frame instead of ending the file;
          // surface that as onEnd so JS loop logic keeps working.
          if (*(int *)prop->data == 0) break;
          dispatch_async(dispatch_get_main_queue(), ^{
            if (self.onEnd) self.onEnd(@{});
          });
        }
        break;
      }
      case MPV_EVENT_END_FILE: {
        mpv_event_end_file *endFile = (mpv_event_end_file *)event->data;
        if (endFile->reason == MPV_END_FILE_REASON_EOF) {
          dispatch_async(dispatch_get_main_queue(), ^{
            if (self.onEnd) self.onEnd(@{});
          });
        }
        break;
      }
      case MPV_EVENT_LOG_MESSAGE: {
        mpv_event_log_message *msg = (mpv_event_log_message *)event->data;
        if (msg->log_level <= MPV_LOG_LEVEL_ERROR) {
          // Event payloads die at the next mpv_wait_event, so the string must
          // be copied on this thread before hopping to the main queue.
          NSString *text = [NSString stringWithUTF8String:msg->text ? msg->text : ""] ?: @"";
          dispatch_async(dispatch_get_main_queue(), ^{
            if (self.onError) {
              self.onError(@{ @"message": text });
            }
          });
        }
        break;
      }
      default:
        break;
    }
  }
  dispatch_semaphore_signal(_eventThreadDone);
}

- (void)emitProgressAt:(double)sec
{
  NSInteger positionMs = (NSInteger)((isfinite(sec) && sec > 0 ? sec : 0) * 1000.0);
  NSInteger durationMs = (NSInteger)((_durationSec > 0 && isfinite(_durationSec) ? _durationSec : 0) * 1000.0);
  dispatch_async(dispatch_get_main_queue(), ^{
    if (self.onProgress) {
      self.onProgress(@{ @"positionMs": @(positionMs), @"durationMs": @(durationMs) });
    }
  });
}

- (NSString *)mediaPathForSrc:(NSString *)src
{
  if ([src hasPrefix:@"file://"]) {
    NSString *raw = [src substringFromIndex:7];
    return [raw stringByRemovingPercentEncoding] ?: raw;
  }
  return src;
}

- (void)setSrc:(NSString *)src
{
  if (_src == src || [_src isEqualToString:src]) return;
  _src = src;
  if (_mpv == NULL || src.length == 0) return;

  _durationSec = 0;
  _lastReportedSec = 0;
  NSString *path = [self mediaPathForSrc:src];
  if (_glLayer.renderContext == NULL) {
    // Not drawable yet; the render-context-ready callback loads it.
    _pendingLoadPath = [path copy];
    return;
  }
  [self loadMediaPath:path];
}

- (void)loadMediaPath:(NSString *)path
{
  if (_mpv == NULL || path.length == 0) return;
  const char *args[] = { "loadfile", path.UTF8String, "replace", NULL };
  if (mpv_command(_mpv, args) < 0 && self.onError) {
    self.onError(@{ @"message": [NSString stringWithFormat:@"无法打开视频：%@", path] });
  }
}

// Called by the layer on its draw thread once the render context exists;
// vo_libmpv must exist before a file's video reconfig runs, so a src that
// arrived before the first draw is loaded from here instead.
- (void)mpvRenderContextReady
{
  NSString *pending = _pendingLoadPath;
  _pendingLoadPath = nil;
  if (pending != nil) [self loadMediaPath:pending];
}

- (void)setPlaying:(BOOL)playing
{
  _playing = playing;
  if (_mpv == NULL) return;
  int paused = playing ? 0 : 1;  // mpv tracks the inverse state as "pause"
  mpv_set_property(_mpv, "pause", MPV_FORMAT_FLAG, &paused);
}

- (void)setRate:(CGFloat)rate
{
  _rate = rate;
  if (_mpv == NULL || rate <= 0) return;
  // scaletempo (mpv default) stretches audio without changing pitch, unlike
  // AVPlayer's plain rate, so 0.6x speech stays intelligible.
  double speed = rate;
  mpv_set_property(_mpv, "speed", MPV_FORMAT_DOUBLE, &speed);
}

- (void)dealloc
{
  _eventThreadStopped = YES;
  if (_mpv != NULL) mpv_wakeup(_mpv);
  if (_eventThread != nil) {
    // The event thread must leave mpv_wait_event before the handle dies.
    dispatch_semaphore_wait(_eventThreadDone, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
  }
  // Stop the layer's drawing thread before freeing what it renders with.
  _glLayer.asynchronous = NO;
  mpv_render_context *context = _glLayer.renderContext;
  _glLayer.renderContext = NULL;
  _glLayer.mpv = NULL;
  if (context != NULL) mpv_render_context_free(context);
  if (_mpv != NULL) mpv_destroy(_mpv);
}

@end

#pragma mark - RN wiring

@interface RNMpvPlayerViewManager : RCTViewManager
@end

@implementation RNMpvPlayerViewManager

// Registered under the same JS name the AVPlayer version used, so
// requireNativeComponent("RNVideoPlayerView") keeps working unchanged.
RCT_EXPORT_MODULE(RNVideoPlayerView)

+ (BOOL)requiresMainQueueSetup
{
  return YES;
}

- (NSView *)view
{
  return [RNMpvPlayerView new];
}

RCT_EXPORT_VIEW_PROPERTY(src, NSString)
RCT_EXPORT_VIEW_PROPERTY(playing, BOOL)
RCT_EXPORT_VIEW_PROPERTY(rate, CGFloat)
RCT_EXPORT_VIEW_PROPERTY(onProgress, RCTDirectEventBlock)
RCT_EXPORT_VIEW_PROPERTY(onEnd, RCTDirectEventBlock)
RCT_EXPORT_VIEW_PROPERTY(onError, RCTDirectEventBlock)

@end

@interface RNMpvVideoControl : NSObject <RCTBridgeModule>
@end

@implementation RNMpvVideoControl

// RCTModuleData injects the bridge via KVC only when the module responds to
// the property; without this synthesis `self.bridge` below crashes with
// "unrecognized selector sent to instance".
@synthesize bridge = _bridge;

RCT_EXPORT_MODULE(RNVideoControl)

// Share the UI manager's queue like RCTViewManager does, so addUIBlock's
// RCTAssertUIManagerQueue passes inside seek.
- (dispatch_queue_t)methodQueue
{
  return self.bridge.uiManager.methodQueue;
}

RCT_EXPORT_METHOD(seek:(nonnull NSNumber *)reactTag ms:(double)ms)
{
  RCTUIManager *uiManager = self.bridge.uiManager;
  if (!uiManager) return;
  double seconds = ms / 1000.0;
  if (seconds < 0) seconds = 0;
  NSString *target = [NSString stringWithFormat:@"%.3f", seconds];
  [uiManager addUIBlock:^(RCTUIManager *manager, NSDictionary<NSNumber *, RCTUIView *> *viewRegistry) {
    RCTUIView *registered = viewRegistry[reactTag];
    if (![registered isKindOfClass:[RNMpvPlayerView class]]) return;
    RNMpvPlayerView *view = (RNMpvPlayerView *)registered;
    if (view.mpvHandle == NULL) return;
    const char *args[] = { "seek", target.UTF8String, "absolute+exact", NULL };
    mpv_command(view.mpvHandle, args);
  }];
}

RCT_EXPORT_METHOD(setVolume:(nonnull NSNumber *)reactTag volume:(nonnull NSNumber *)volume)
{
  RCTUIManager *uiManager = self.bridge.uiManager;
  if (!uiManager) return;
  [uiManager addUIBlock:^(RCTUIManager *manager, NSDictionary<NSNumber *, RCTUIView *> *viewRegistry) {
    RCTUIView *registered = viewRegistry[reactTag];
    if (![registered isKindOfClass:[RNMpvPlayerView class]]) return;
    RNMpvPlayerView *view = (RNMpvPlayerView *)registered;
    if (view.mpvHandle == NULL) return;
    double level = volume.doubleValue;  // UI range 0-100; mpv accepts up to 130
    mpv_set_property(view.mpvHandle, "volume", MPV_FORMAT_DOUBLE, &level);
  }];
}

RCT_EXPORT_METHOD(getSubtitleTracks:(nonnull NSNumber *)reactTag
                           resolver:(RCTPromiseResolveBlock)resolve
                           rejecter:(RCTPromiseRejectBlock)reject)
{
  RCTUIManager *uiManager = self.bridge.uiManager;
  if (!uiManager) { resolve(@[]); return; }
  [uiManager addUIBlock:^(RCTUIManager *manager, NSDictionary<NSNumber *, RCTUIView *> *viewRegistry) {
    RCTUIView *registered = viewRegistry[reactTag];
    if (![registered isKindOfClass:[RNMpvPlayerView class]] || ((RNMpvPlayerView *)registered).mpvHandle == NULL) {
      resolve(@[]);
      return;
    }
    mpv_handle *mpv = ((RNMpvPlayerView *)registered).mpvHandle;
    NSMutableArray *tracks = [NSMutableArray array];
    mpv_node node;
    if (mpv_get_property(mpv, "track-list", MPV_FORMAT_NODE, &node) >= 0) {
      if (node.format == MPV_FORMAT_NODE_ARRAY) {
        for (int i = 0; i < node.u.list->num; i++) {
          mpv_node_list *entry = node.u.list->values[i].u.list;
          NSString *type = @"";
          NSString *lang = @"";
          NSString *title = @"";
          NSString *codec = @"";
          NSString *externalFilename = @"";
          int64_t trackId = 0;
          int64_t subStreamIndex = -1;
          BOOL selected = NO;
          BOOL external = NO;
          for (int j = 0; j < entry->num; j++) {
            NSString *key = @(entry->keys[j]);
            mpv_node *value = &entry->values[j];
            if ([key isEqualToString:@"type"]) type = @(value->u.string);
            else if ([key isEqualToString:@"id"]) trackId = value->u.int64;
            else if ([key isEqualToString:@"lang"] && value->format == MPV_FORMAT_STRING) lang = @(value->u.string);
            else if ([key isEqualToString:@"title"] && value->format == MPV_FORMAT_STRING) title = @(value->u.string);
            else if ([key isEqualToString:@"codec"]) codec = @(value->u.string);
            else if ([key isEqualToString:@"selected"]) selected = value->u.flag != 0;
            else if ([key isEqualToString:@"external"]) external = value->u.flag != 0;
            else if ([key isEqualToString:@"external-filename"] && value->format == MPV_FORMAT_STRING) externalFilename = @(value->u.string);
            else if ([key isEqualToString:@"sub-stream-index"] && value->format == MPV_FORMAT_INT64) subStreamIndex = value->u.int64;
          }
          if (![type isEqualToString:@"sub"]) continue;
          // VobSub tracks carry the .idx filename as title; trim it so the
          // picker shows a readable name.
          NSString *name = title.length > 0 ? title : lang;
          name = [name stringByReplacingOccurrencesOfString:@".idx" withString:@""];
          if (name.length == 0) name = [NSString stringWithFormat:@"字幕 %lld", trackId];
          if (lang.length > 0) name = [name stringByAppendingFormat:@" (%@)", lang];
          // mpv's secondary-sid only renders text subtitles; bitmap formats
          // (VobSub, PGS) are excluded from the top-slot picker.
          BOOL textTrack = !([codec isEqualToString:@"dvd_subtitle"] ||
                             [codec isEqualToString:@"hdmv_pgs_subtitle"]);
          [tracks addObject:@{
            @"id": @(trackId),
            @"name": name,
            @"lang": lang,
            @"codec": codec,
            @"selected": @(selected),
            @"textTrack": @(textTrack),
            @"external": @(external),
            @"externalFilename": externalFilename,
            @"subStreamIndex": @(subStreamIndex),
          }];
        }
      }
      mpv_free_node_contents(&node);
    }
    int64_t secondarySid = 0;
    mpv_get_property(mpv, "secondary-sid", MPV_FORMAT_INT64, &secondarySid);
    resolve(@{ @"tracks": tracks, @"secondaryId": @(secondarySid) });
  }];
}

RCT_EXPORT_METHOD(selectSubtitle:(nonnull NSNumber *)reactTag trackId:(nonnull NSNumber *)trackId)
{
  RCTUIManager *uiManager = self.bridge.uiManager;
  if (!uiManager) return;
  [uiManager addUIBlock:^(RCTUIManager *manager, NSDictionary<NSNumber *, RCTUIView *> *viewRegistry) {
    RCTUIView *registered = viewRegistry[reactTag];
    if (![registered isKindOfClass:[RNMpvPlayerView class]]) return;
    RNMpvPlayerView *view = (RNMpvPlayerView *)registered;
    if (view.mpvHandle == NULL) return;
    int64_t sid = trackId.longLongValue;  // 0 turns subtitles off
    mpv_set_property(view.mpvHandle, "sid", MPV_FORMAT_INT64, &sid);
  }];
}

RCT_EXPORT_METHOD(selectSecondarySubtitle:(nonnull NSNumber *)reactTag trackId:(nonnull NSNumber *)trackId)
{
  RCTUIManager *uiManager = self.bridge.uiManager;
  if (!uiManager) return;
  [uiManager addUIBlock:^(RCTUIManager *manager, NSDictionary<NSNumber *, RCTUIView *> *viewRegistry) {
    RCTUIView *registered = viewRegistry[reactTag];
    if (![registered isKindOfClass:[RNMpvPlayerView class]]) return;
    RNMpvPlayerView *view = (RNMpvPlayerView *)registered;
    if (view.mpvHandle == NULL) return;
    // Rendered at the top of the frame; mpv requires a text track here.
    int64_t sid = trackId.longLongValue;  // 0 turns the top subtitle off
    mpv_set_property(view.mpvHandle, "secondary-sid", MPV_FORMAT_INT64, &sid);
  }];
}

@end
