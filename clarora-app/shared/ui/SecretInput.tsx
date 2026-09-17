import { Platform, TextInput, requireNativeComponent, type NativeSyntheticEvent, type TextInputProps, type ViewProps } from 'react-native';

type NativeProps = ViewProps & {
  value: string;
  editable: boolean;
  placeholder?: string;
  textColor: string;
  onChange: (event: NativeSyntheticEvent<{ text: string }>) => void;
};
const MacSecureInput = Platform.OS === 'macos'
  ? requireNativeComponent<NativeProps>('RNMacSecureInput')
  : null;

export function SecretInput({ value = '', onChangeText, editable = true, textColor, ...props }: TextInputProps & { textColor: string }) {
  if (MacSecureInput) {
    return <MacSecureInput accessibilityLabel={props.accessibilityLabel}
      placeholder={props.placeholder} value={value} editable={editable}
      textColor={textColor} style={{ height: 24, width: '100%' }}
      onChange={event => onChangeText?.(event.nativeEvent.text)} />;
  }
  return <TextInput {...props} value={value} editable={editable} onChangeText={onChangeText}
    secureTextEntry style={{ color: textColor, padding: 0, minHeight: 24 }} />;
}
