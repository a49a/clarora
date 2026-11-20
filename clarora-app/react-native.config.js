// react-native-video ships Android/iOS players only. Keeping it out of the
// deprecated macOS/Windows RN autolinking avoids pod install failures there;
// GPUI remains the desktop client.
module.exports = {
  dependencies: {
    "react-native-video": {
      platforms: {
        macos: null,
        windows: null,
      },
    },
  },
};
