import { Platform, requireNativeComponent, type NativeSyntheticEvent, type ViewProps, type processColor } from 'react-native';

type NativeColor = ReturnType<typeof processColor>;
export type NativeSelectableSubtitleProps = ViewProps & {
  text: string;
  fontSize?: number;
  selectionEnabled?: boolean;
  textColor?: NativeColor;
  activeCueStart?: number;
  activeCueLength?: number;
  activeWordStart?: number;
  activeWordLength?: number;
  activeCueTextColor?: NativeColor;
  activeAccentColor?: NativeColor;
  activeWordTextColor?: NativeColor;
  loopStartMarkerIndex?: number;
  loopStartMarkerAfter?: boolean;
  loopEndMarkerIndex?: number;
  loopEndMarkerAfter?: boolean;
  keyRangesJson?: string;
  keyHighlightColor?: NativeColor;
  onAskSelection?: (event: NativeSyntheticEvent<{ text: string }>) => void;
  onTapAtCharacter?: (event: NativeSyntheticEvent<{ index: number }>) => void;
};

// Register once per JS runtime; every screen must import this shared binding.
export const NativeSelectableSubtitleView = Platform.OS === 'macos'
  ? requireNativeComponent<NativeSelectableSubtitleProps>('RNSelectableSubtitleView')
  : null;
