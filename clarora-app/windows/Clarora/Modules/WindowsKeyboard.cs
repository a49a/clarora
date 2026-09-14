using Microsoft.ReactNative.Managed;
using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Windows.System;
using Windows.UI.Core;
using Windows.UI.Xaml;
using Windows.UI.Xaml.Controls;
using Windows.UI.Xaml.Controls.Primitives;
using Windows.UI.Xaml.Input;

namespace Clarora
{
    [ReactModule("RNKeyboard")]
    public sealed class WindowsKeyboard : IDisposable
    {
        private ReactContext context;
        private CoreWindow window;
        private readonly Queue<string> keys = new Queue<string>();
        private TaskCompletionSource<string> waiting;
        [ReactInitializer] public void Initialize(ReactContext value) { context = value; }
        [ReactMethod("startListening")] public void Start() => context.UIDispatcher.Post(() => {
            StopOnUI(); window = Window.Current.CoreWindow; window.KeyDown += KeyDown;
        });
        private void KeyDown(CoreWindow sender, KeyEventArgs e)
        {
            var focused = FocusManager.GetFocusedElement();
            if (focused is TextBox || focused is PasswordBox ||
                (focused is ButtonBase && e.VirtualKey != VirtualKey.A && e.VirtualKey != VirtualKey.D && e.VirtualKey != VirtualKey.P)) return;
            if ((sender.GetKeyState(VirtualKey.Control) & CoreVirtualKeyStates.Down) != 0 ||
                (sender.GetKeyState(VirtualKey.Menu) & CoreVirtualKeyStates.Down) != 0) return;
            string key = null;
            switch (e.VirtualKey) {
                case VirtualKey.Left: key = "ArrowLeft"; break;
                case VirtualKey.Right: key = "ArrowRight"; break;
                case VirtualKey.Up: key = "ArrowUp"; break;
                case VirtualKey.Down: key = "ArrowDown"; break;
                case VirtualKey.Space:
                    if (e.KeyStatus.WasKeyDown) { e.Handled = true; return; }
                    key = "Space"; break;
                case VirtualKey.Enter:
                    if (e.KeyStatus.WasKeyDown) { e.Handled = true; return; }
                    key = "Enter"; break;
                case VirtualKey.A:
                    if (e.KeyStatus.WasKeyDown) { e.Handled = true; return; }
                    key = "a"; break;
                case VirtualKey.D:
                    if (e.KeyStatus.WasKeyDown) { e.Handled = true; return; }
                    key = "d"; break;
                case VirtualKey.P:
                    if (e.KeyStatus.WasKeyDown) { e.Handled = true; return; }
                    key = "p"; break;
                case VirtualKey.Number1: key = "1"; break;
                case VirtualKey.Number2: key = "2"; break;
                case VirtualKey.Number3: key = "3"; break;
            }
            if (key == null) return;
            e.Handled = true;
            if (waiting != null) { var request = waiting; waiting = null; request.TrySetResult(key); }
            else if (keys.Count < 16) keys.Enqueue(key);
        }
        [ReactMethod("getNextKey")]
        public Task<string> Next()
        {
            var request = new TaskCompletionSource<string>();
            context.UIDispatcher.Post(() => {
                if (window == null) request.TrySetResult(null);
                else if (keys.Count > 0) request.TrySetResult(keys.Dequeue());
                else { waiting?.TrySetResult(null); waiting = request; }
            });
            return request.Task;
        }
        private void StopOnUI() {
            if (window != null) window.KeyDown -= KeyDown;
            window = null; keys.Clear(); waiting?.TrySetResult(null); waiting = null;
        }
        [ReactMethod("stopListening")] public void Stop() => context.UIDispatcher.Post(StopOnUI);
        public void Dispose() { Stop(); }
    }
}
