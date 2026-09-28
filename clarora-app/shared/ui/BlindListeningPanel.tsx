import { Platform, Pressable, StyleSheet, Text, View } from 'react-native';
import { useAppTheme } from './ThemeContext';

// 盲听内联卡片:隐藏态 / 查看态 / 播放结束态。
// 设计 internal-docs/blind-listening-design.md §3.2/§3.3/§5。
// 展示组件不持有播放状态;所有动作经 props 回调到听力页统一处理路径。

export interface RevealedTextSnapshot {
  text: string;
  timeRange: string;
  sparse: boolean;
}

export type RevealDisabledReason =
  | 'no-subtitles'    // 没有字幕:换成非交互说明 + 管理字幕入口
  | 'before-first'    // 第一段字幕开始前
  | 'pending'         // 揭晓请求进行中
  | 'switching'       // 音频/字幕切换中
  | 'unavailable';    // 时长/位置尚不可用

export function BlindListeningPanel({
  audioTitle,
  hasSubtitles,
  revealDisabledReason,
  revealed,
  finished,
  onBackFive,
  onReveal,
  onHideText,
  onResumeBlind,
  onRestart,
  onViewFull,
  onManageSubtitles,
}: {
  audioTitle: string | null;
  hasSubtitles: boolean;
  revealDisabledReason: RevealDisabledReason | null;
  revealed: RevealedTextSnapshot | null;
  finished: boolean;
  onBackFive: () => void;
  onReveal: () => void;
  onHideText: () => void;
  onResumeBlind: () => void;
  onRestart: () => void;
  onViewFull: () => void;
  onManageSubtitles: () => void;
}) {
  const { theme } = useAppTheme();
  const styles = makeStyles(theme);
  const minHeight = Platform.OS === 'android' ? 48 : Platform.OS === 'ios' ? 44 : 36;

  const action = (label: string, onPress: () => void, opts: { primary?: boolean; disabled?: boolean; accessibilityLabel?: string } = {}) => (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={opts.accessibilityLabel ?? label}
      disabled={opts.disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.actionBtn,
        opts.primary && styles.actionBtnPrimary,
        opts.disabled && styles.actionBtnDisabled,
        pressed && styles.actionBtnPressed,
        { minHeight },
      ]}
    >
      <Text style={[styles.actionBtnText, opts.primary && styles.actionBtnTextPrimary, opts.disabled && styles.actionBtnTextDisabled]}>
        {label}
      </Text>
    </Pressable>
  );

  const revealBlocked = revealDisabledReason != null;
  const revealReasonText: string | null =
    revealDisabledReason === 'no-subtitles' ? '暂无字幕，暂不能核对原文'
    : revealDisabledReason === 'before-first' ? '播放到有字幕的部分后可查看'
    : revealDisabledReason === 'pending' ? '揭晓中…'
    : revealDisabledReason === 'switching' ? '音频切换中…'
    : revealDisabledReason === 'unavailable' ? '稍候可用'
    : null;

  return (
    <View
      style={styles.card}
      accessibilityLabel="盲听模式，字幕已隐藏"
    >
      {audioTitle ? <Text style={styles.title} numberOfLines={1}>{audioTitle}</Text> : null}

      {finished ? (
        <>
          <Text style={styles.headline}>听完了</Text>
          <View style={styles.actionRow}>
            {action('从头再听', onRestart, { primary: true })}
            {action('查看全文', onViewFull)}
          </View>
        </>
      ) : revealed ? (
        <>
          <Text accessibilityRole="header" style={styles.revealHeading}>
            {revealed.sparse ? '上一条字幕' : '刚才一句'}
          </Text>
          <Text style={styles.revealTime}>{revealed.timeRange}</Text>
          <Text style={styles.revealText} selectable>{revealed.text}</Text>
          <View style={styles.actionRow}>
            {action('收起文字', onHideText)}
            {action('继续盲听', onResumeBlind, { primary: true })}
          </View>
          <Pressable accessibilityRole="link" onPress={onViewFull} style={styles.secondaryLink}>
            <Text style={styles.secondaryLinkText}>查看全文（切到字幕模式，保持暂停）</Text>
          </Pressable>
        </>
      ) : (
        <>
          <Text style={styles.headline}>{hasSubtitles ? '先听，不看字幕' : '专心听音频'}</Text>
          <Text style={styles.hint}>{hasSubtitles ? '在心里回想刚才听到的内容' : '没有字幕也可以盲听'}</Text>
          <View style={styles.actionRow}>
            {action('回退 5 秒', onBackFive)}
            {hasSubtitles
              ? action('看刚才一句', onReveal, { primary: true, disabled: revealBlocked, accessibilityLabel: revealReasonText ?? '看刚才一句' })
              : null}
          </View>
          {hasSubtitles ? (
            revealBlocked && revealDisabledReason !== 'pending' ? (
              <Text style={styles.note}>{revealReasonText}</Text>
            ) : null
          ) : (
            <View style={styles.noteRow}>
              <Text style={styles.note}>{revealReasonText}</Text>
              <Pressable accessibilityRole="link" onPress={onManageSubtitles} style={styles.secondaryLink}>
                <Text style={styles.secondaryLinkText}>管理字幕</Text>
              </Pressable>
            </View>
          )}
        </>
      )}
    </View>
  );
}

function makeStyles(theme: ReturnType<typeof useAppTheme>['theme']) {
  return StyleSheet.create({
    card: {
      alignSelf: 'stretch',
      alignItems: 'center',
      justifyContent: 'center',
      borderRadius: 16,
      backgroundColor: theme.surface,
      borderWidth: 1,
      borderColor: theme.border,
      paddingVertical: 28,
      paddingHorizontal: 18,
      gap: 12,
      flexGrow: 1,
      flexShrink: 1,
    },
    title: {
      color: theme.textSecondary,
      fontSize: 13,
      fontWeight: '600',
      marginBottom: 4,
    },
    headline: {
      color: theme.text,
      fontSize: 20,
      fontWeight: '700',
      textAlign: 'center',
    },
    hint: {
      color: theme.textSecondary,
      fontSize: 14,
      textAlign: 'center',
    },
    actionRow: {
      flexDirection: 'row',
      flexWrap: 'wrap',
      justifyContent: 'center',
      gap: 10,
      marginTop: 6,
    },
    actionBtn: {
      borderRadius: 12,
      paddingHorizontal: 16,
      paddingVertical: 10,
      justifyContent: 'center',
      backgroundColor: theme.surfaceHover,
      borderWidth: 1,
      borderColor: theme.border,
    },
    actionBtnPrimary: {
      backgroundColor: theme.accent,
      borderColor: theme.accent,
    },
    actionBtnDisabled: {
      opacity: 0.5,
    },
    actionBtnPressed: {
      opacity: 0.7,
    },
    actionBtnText: {
      color: theme.accent,
      fontSize: 14,
      fontWeight: '600',
      textAlign: 'center',
    },
    actionBtnTextPrimary: {
      color: '#fff',
    },
    actionBtnTextDisabled: {
      color: theme.textMuted,
    },
    revealHeading: {
      color: theme.text,
      fontSize: 15,
      fontWeight: '700',
      alignSelf: 'flex-start',
    },
    revealTime: {
      color: theme.textMuted,
      fontSize: 12,
      alignSelf: 'flex-start',
    },
    revealText: {
      color: theme.text,
      fontSize: 17,
      lineHeight: 26,
      alignSelf: 'stretch',
    },
    note: {
      color: theme.textMuted,
      fontSize: 12,
      textAlign: 'center',
    },
    noteRow: {
      alignItems: 'center',
      gap: 6,
    },
    secondaryLink: {
      marginTop: 4,
      paddingVertical: 6,
      paddingHorizontal: 8,
    },
    secondaryLinkText: {
      color: theme.accent,
      fontSize: 13,
      textDecorationLine: 'underline',
      textAlign: 'center',
    },
  });
}
