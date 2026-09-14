// Windows uses the app's own storage/media modules. The installed community
// packages contain legacy Windows projects which target a different RNW ABI.
module.exports = {
  dependencies: {
    'react-native-fs': { platforms: { windows: null } },
    'react-native-sound': { platforms: { windows: null } },
    'react-native-sqlite-storage': { platforms: { windows: null } },
  },
};
