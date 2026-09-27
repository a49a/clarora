// 听力页设置弹层遮罩命中决策:纯函数,回归测试见 tests/listening-settings-mask.test.cjs。
// AI 忙碌(aiBusy)时练习组/音频入口按钮本身已禁用,遮罩命中须走同样守卫,只收起设置、不打开选择列表。

export type StudySettingsKey = 'subtitle' | 'speed';
export type StudySettingsTrigger = StudySettingsKey | 'ai' | 'practice' | 'audio' | 'exit';

export interface StudySettingsMaskState {
  activeSettings: StudySettingsKey | null;
  aiBusy: boolean;
  hasSelectedPractice: boolean;
}

export type StudySettingsMaskAction =
  | { kind: 'close' }
  | { kind: 'open'; settings: StudySettingsKey }
  | { kind: 'ai' }
  | { kind: 'practice' }
  | { kind: 'audio' }
  | { kind: 'exit' }
  | { kind: 'dismiss' };

export function resolveStudySettingsMaskTap(
  trigger: StudySettingsTrigger,
  state: StudySettingsMaskState,
): StudySettingsMaskAction {
  if (trigger === 'subtitle' || trigger === 'speed') {
    return state.activeSettings === trigger
      ? { kind: 'close' }
      : { kind: 'open', settings: trigger };
  }
  if (trigger === 'ai') return { kind: 'ai' };
  if (trigger === 'practice') return state.aiBusy ? { kind: 'dismiss' } : { kind: 'practice' };
  if (trigger === 'audio') {
    return state.aiBusy || !state.hasSelectedPractice ? { kind: 'dismiss' } : { kind: 'audio' };
  }
  return { kind: 'exit' };
}
