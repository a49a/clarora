const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

test('macOS secure input emits typed, replaced and cleared text through its actual AppKit editor',
  { skip: process.platform !== 'darwin' }, () => {
    const source = fs.readFileSync(path.join(__dirname, '../macos/Clarora-macOS/AppDelegate.mm'), 'utf8');
    const start = source.indexOf('@interface RNMacSecureInputView');
    const end = source.indexOf('@interface RNMacSecureInputManager', start);
    assert.ok(start >= 0 && end > start);
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'clarora-secure-input-'));
    try {
      const file = path.join(directory, 'test.m');
      const binary = path.join(directory, 'test');
      fs.writeFileSync(file, `#import <AppKit/AppKit.h>
typedef void (^RCTBubblingEventBlock)(NSDictionary *);
${source.slice(start, end)}
int main(void) {
  @autoreleasepool {
    [NSApplication sharedApplication];
    NSWindow *window = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 360, 90)
      styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
    RNMacSecureInputView *view = [[RNMacSecureInputView alloc] initWithFrame:NSMakeRect(10, 20, 300, 24)];
    [window.contentView addSubview:view];
    NSMutableArray *events = [NSMutableArray new];
    view.onChange = ^(NSDictionary *event) { [events addObject:event[@"text"]]; };
    [window makeKeyAndOrderFront:nil];
    [view.field selectText:nil];
    NSTextView *editor = (NSTextView *)view.field.currentEditor;
    NSCAssert(editor != nil, @"Secure field editor must be active");
    [editor insertText:@"dummy-key" replacementRange:NSMakeRange(0, 0)];
    NSCAssert([events.lastObject isEqual:@"dummy-key"], @"Typing must reach the change callback");
    NSRange selection = editor.selectedRange;
    view.value = @"dummy-key";
    NSCAssert(NSEqualRanges(selection, editor.selectedRange), @"Controlled echo must preserve selection");
    [editor insertText:@"replacement" replacementRange:NSMakeRange(0, editor.string.length)];
    NSCAssert([events.lastObject isEqual:@"replacement"], @"Replacement must reach the change callback");
    [editor insertText:@"" replacementRange:NSMakeRange(0, editor.string.length)];
    NSCAssert([events.lastObject isEqual:@""], @"Clearing must reach the change callback");
    [window makeFirstResponder:nil];
    view.value = @"restored-key";
    NSCAssert([view.field.stringValue isEqual:@"restored-key"], @"Saved value must restore");
    [window orderOut:nil];
  }
  return 0;
}`);
      const compile = spawnSync('xcrun', ['clang', '-fobjc-arc', '-fblocks', '-framework', 'AppKit', file, '-o', binary], { encoding: 'utf8', timeout: 60000 });
      assert.equal(compile.status, 0, compile.stderr);
      const run = spawnSync(binary, [], { encoding: 'utf8', timeout: 15000 });
      assert.equal(run.status, 0, run.stderr);
    } finally {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });
