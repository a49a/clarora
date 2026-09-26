import { useCallback, useEffect, useId, useMemo, useRef, useState, type MutableRefObject, type ReactNode } from 'react';
import { AccessibilityInfo, Dimensions, findNodeHandle, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useAppTheme } from './ThemeContext';
import { usePopoverPortal, type PortalGeometry } from './PopoverRoot';

const NARROW_BREAKPOINT = 600;
const PANEL_WIDTH = 380;
const SCREEN_MARGIN = 12;
const MIN_PANEL_HEIGHT = 130;
const TOUCH_HEIGHT_TOUCH = 44;
const TOUCH_HEIGHT_POINTER = 34;

type Rect = { x: number; y: number; width: number; height: number };
type PanelLayout =
  | { mode: 'sheet'; maskColor: string; panel: object }
  | { mode: 'anchored'; maskColor: string; panel: object };

/**
 * 设置弹层(受控开关):open/onVisibilityChange 语义不变。
 *
 * 定位与关闭规则(设计见 internal-docs/listening-settings-popover-design.md):
 * - 桌面面板与入口同层以保证原生鼠标命中;窄屏面板挂到 PopoverRoot;
 * - 可见宽度 ≥600:380 宽锚定浮层,字幕优先下方右对齐、倍速优先上方左对齐,
 *   左右至少 12 边距,首选方向放不下翻转到另一侧,两侧都紧取较大一侧滚动;
 * - 可见宽度 <600:底部面板(宽 = 可见 - 24,高 ≤ 可见 80%),半透明遮罩,
 *   标题与「完成」常驻,内容独立滚动;
 * - 点外遮罩只关闭面板,不触发底层控件;桌面 Esc 关闭,
 *   Android 返回键由听力页 BackHandler 处理。
 */
export function StudyOptions({ label, title, children, open, onVisibilityChange, direction = 'down', align = 'left', badge, triggerRef, onBackdropPress, suppressReturnFocus = false }: {
  open: boolean;
  onVisibilityChange: (visible: boolean) => void;
  direction?: 'down' | 'up';
  align?: 'left' | 'right';
  /** 入口的动态说明(如「当前 1×」),并入显示与可访问名称 */
  badge?: string;
  triggerRef?: MutableRefObject<View | null>;
  onBackdropPress?: (pageX: number, pageY: number) => Promise<boolean>;
  suppressReturnFocus?: boolean;
  label: string; title: string; children: ReactNode | ((close: () => void) => ReactNode);
}) {
  const { theme } = useAppTheme();
  const portal = usePopoverPortal();
  const portalId = useId();
  const anchorRef = useRef<View | null>(null);
  const panelRef = useRef<View | null>(null);
  const titleRef = useRef<Text | null>(null);
  const wasOpenRef = useRef(false);
  const returnFocusRef = useRef(true);
  const [geometry, setGeometry] = useState<{ anchor: Rect; layer: PortalGeometry; panelHeight?: number } | null>(null);
  const close = useCallback(() => {
    returnFocusRef.current = true;
    onVisibilityChange(false);
  }, [onVisibilityChange]);
  const handleBackdropPress = (pageX: number, pageY: number) => {
    if (!onBackdropPress) { close(); return; }
    returnFocusRef.current = false;
    void onBackdropPress(pageX, pageY).then(handled => {
      if (!handled) close();
    }).catch(close);
  };

  useEffect(() => {
    if (open) {
      wasOpenRef.current = true;
      returnFocusRef.current = true;
      return;
    }
    if (!wasOpenRef.current) return;
    wasOpenRef.current = false;
    if (!returnFocusRef.current || suppressReturnFocus) {
      returnFocusRef.current = true;
      return;
    }
    const frame = requestAnimationFrame(() => {
      anchorRef.current?.focus?.();
      const tag = findNodeHandle(anchorRef.current);
      if (tag != null) AccessibilityInfo.setAccessibilityFocus(tag);
    });
    return () => cancelAnimationFrame(frame);
  }, [open, suppressReturnFocus]);

  // 打开或窗口/旋转变化时重测:入口与页面层用同一窗口坐标系。
  useEffect(() => {
    if (!open) { setGeometry(null); return; }
    if (!portal) return;
    let cancelled = false;
    const measure = () => {
      if (!anchorRef.current) return;
      portal.measureLayer().then(layer => {
        if (cancelled) return;
        if (layer.width <= 0 || layer.height <= 0) return;
        anchorRef.current?.measureInWindow((x, y, width, height) => {
          if (cancelled) return;
          setGeometry(previous => ({ anchor: { x, y, width, height }, layer, panelHeight: previous?.panelHeight }));
        });
      }).catch(() => {});
    };
    measure();
    const subscription = Dimensions.addEventListener('change', measure);
    return () => { cancelled = true; subscription?.remove(); };
  }, [open, portal]);

  const layout: PanelLayout | null = useMemo(() => {
    if (!geometry) return null;
    const { anchor, layer, panelHeight } = geometry;
    const anchorTop = anchor.y - layer.y;
    const anchorBottom = anchorTop + anchor.height;
    const maskColor = layer.width < NARROW_BREAKPOINT ? 'rgba(0,0,0,0.35)' : 'transparent';
    if (layer.width < NARROW_BREAKPOINT) {
      return {
        mode: 'sheet', maskColor,
        panel: {
          position: 'absolute', left: SCREEN_MARGIN, right: SCREEN_MARGIN,
          bottom: SCREEN_MARGIN,
          maxHeight: Math.max(0, Math.min(layer.height - SCREEN_MARGIN * 2, Math.round(layer.height * 0.8))),
        },
      };
    }
    const panelWidth = Math.min(PANEL_WIDTH, Math.max(layer.width - SCREEN_MARGIN * 2, 200));
    const maxX = Math.max(SCREEN_MARGIN, layer.width - SCREEN_MARGIN - panelWidth);
    const anchorLeft = anchor.x - layer.x;
    const left = Math.min(Math.max(align === 'right' ? anchorLeft + anchor.width - panelWidth : anchorLeft, SCREEN_MARGIN), maxX);
    const anchorTopInLayer = anchorTop;
    const anchorBottomInLayer = anchorBottom;
    const spaceAbove = anchorTopInLayer - SCREEN_MARGIN;
    const spaceBelow = layer.height - anchorBottomInLayer - SCREEN_MARGIN;
    const preferDown = direction === 'down';
    const placeAbove = (maxHeight: number) => panelHeight == null
      ? { bottom: layer.height - anchorTopInLayer + 6 }
      : { top: Math.max(SCREEN_MARGIN, anchorTopInLayer - 6 - Math.min(panelHeight, maxHeight)) };
    let position: { top: number } | { bottom: number };
    let maxPanelHeight: number;
    if (preferDown) {
      if (spaceBelow >= MIN_PANEL_HEIGHT) { position = { top: anchorBottomInLayer + 6 }; maxPanelHeight = spaceBelow - 6; }
      else if (spaceAbove >= MIN_PANEL_HEIGHT) { maxPanelHeight = Math.min(spaceAbove - 6, 480); position = placeAbove(maxPanelHeight); }
      else { const larger = Math.max(spaceAbove, spaceBelow); maxPanelHeight = Math.max(0, larger - 6); position = larger === spaceBelow ? { top: anchorBottomInLayer + 6 } : placeAbove(maxPanelHeight); }
    } else {
      if (spaceAbove >= MIN_PANEL_HEIGHT) { maxPanelHeight = Math.min(spaceAbove - 6, 480); position = placeAbove(maxPanelHeight); }
      else if (spaceBelow >= MIN_PANEL_HEIGHT) { position = { top: anchorBottomInLayer + 6 }; maxPanelHeight = spaceBelow - 6; }
      else { const larger = Math.max(spaceAbove, spaceBelow); maxPanelHeight = Math.max(0, larger - 6); position = larger === spaceBelow ? { top: anchorBottomInLayer + 6 } : placeAbove(maxPanelHeight); }
    }
    return {
      mode: 'anchored', maskColor: 'transparent',
      panel: { position: 'absolute' as const, ...position, left, width: panelWidth, maxHeight: maxPanelHeight },
    };
  }, [geometry, direction, align]);
  const inlineDesktop = (Platform.OS === 'macos' || Platform.OS === 'windows') && Dimensions.get('window').width >= NARROW_BREAKPOINT;

  const renderPanel = (panelStyle: object, mode: 'sheet' | 'anchored') => (
    <View
      ref={panelRef}
      onLayout={event => {
        const height = event.nativeEvent.layout.height;
        setGeometry(previous => previous && previous.panelHeight !== height ? { ...previous, panelHeight: height } : previous);
      }}
      style={[styles.panel, panelStyle, mode === 'sheet' ? styles.sheet : styles.anchored, {
        backgroundColor: theme.surface, borderColor: theme.border,
      }]}
      {...(Platform.OS === 'macos' || Platform.OS === 'windows' ? ({ tabIndex: -1, onKeyDown: (event: { nativeEvent: { key: string } }) => { if (event.nativeEvent.key === 'Escape') close(); } } as Record<string, unknown>) : {})}
    >
      <View style={[styles.panelHead, { borderBottomColor: theme.border }]}>
        <Text ref={titleRef} accessibilityRole="header" style={[styles.panelTitle, { color: theme.text }]}>{title}</Text>
        <Pressable accessibilityRole="button" accessibilityLabel="完成" onPress={close} style={styles.panelDone} hitSlop={8}>
          <Text style={[styles.panelDoneText, { color: theme.accent }]}>完成</Text>
        </Pressable>
      </View>
      <ScrollView style={styles.panelBodyScroll} contentContainerStyle={styles.panelBody} keyboardShouldPersistTaps="handled">
        {typeof children === 'function' ? children(close) : children}
      </ScrollView>
    </View>
  );

  useEffect(() => {
    if (!open || (!inlineDesktop && (!layout || !portal))) return;
    const frame = requestAnimationFrame(() => {
      panelRef.current?.focus?.();
      const tag = findNodeHandle(titleRef.current);
      if (tag != null) AccessibilityInfo.setAccessibilityFocus(tag);
    });
    return () => cancelAnimationFrame(frame);
  }, [open, layout, portal, inlineDesktop]);

  useEffect(() => () => portal?.unmount(portalId), [portal, portalId]);

  // 面板挂载到页面层;布局就绪才渲染,避免首帧闪现到错误位置。
  useEffect(() => {
    if (!portal) return;
    if (!open || !layout || inlineDesktop) {
      portal.unmount(portalId);
      return;
    }
    portal.mount(portalId, (
      <>
        <Pressable
          accessibilityRole="button" accessibilityLabel="关闭设置面板"
          style={[StyleSheet.absoluteFill, { backgroundColor: layout.maskColor, zIndex: 0 }]}
          onPress={event => handleBackdropPress(event.nativeEvent.pageX, event.nativeEvent.pageY)}
        />
        {renderPanel(layout.panel, layout.mode)}
      </>
    ));
  }, [open, portal, portalId, layout, inlineDesktop, renderPanel, handleBackdropPress]);

  const entryLabel = badge ? `${label}（${badge}）` : label;
  const inlinePanelWidth = geometry ? Math.min(PANEL_WIDTH, Math.max(0, geometry.layer.width - SCREEN_MARGIN * 2)) : PANEL_WIDTH;
  const inlineAnchorLeft = geometry ? geometry.anchor.x - geometry.layer.x : 0;
  const inlinePreferredLeft = align === 'right' && geometry
    ? inlineAnchorLeft + geometry.anchor.width - inlinePanelWidth
    : inlineAnchorLeft;
  const inlinePanelLeft = geometry
    ? Math.min(Math.max(inlinePreferredLeft, SCREEN_MARGIN), geometry.layer.width - SCREEN_MARGIN - inlinePanelWidth) - inlineAnchorLeft
    : 0;
  return (
    <View style={{ position: 'relative', zIndex: open && inlineDesktop ? 50 : undefined }}>
    <Pressable
      ref={node => { anchorRef.current = node; if (triggerRef) triggerRef.current = node; }}
      accessibilityRole="button"
      accessibilityLabel={entryLabel}
      accessibilityState={{ expanded: open }}
      onPress={() => { if (open) close(); else onVisibilityChange(true); }}
      style={({ pressed }) => ({ minHeight: Platform.OS === 'android' ? 48 : Platform.OS === 'ios' ? TOUCH_HEIGHT_TOUCH : TOUCH_HEIGHT_POINTER,
        paddingHorizontal: 12, justifyContent: 'center', borderRadius: 8, backgroundColor: theme.surfaceHover, opacity: pressed ? .7 : 1 })}
    >
      <Text style={{ color: theme.textSecondary, fontSize: 13, fontWeight: '600' }}>{entryLabel} {direction === 'down' ? '▾' : '▴'}</Text>
    </Pressable>
    {open && inlineDesktop && geometry && <Pressable
      accessibilityRole="button" accessibilityLabel="关闭设置面板"
      style={{ position: 'absolute', left: -geometry.anchor.x, top: -geometry.anchor.y,
        width: Dimensions.get('window').width, height: Dimensions.get('window').height, zIndex: 0 }}
      onPress={event => handleBackdropPress(event.nativeEvent.pageX, event.nativeEvent.pageY)}
    />}
    {open && inlineDesktop && geometry && renderPanel({
      position: 'absolute', left: inlinePanelLeft, width: inlinePanelWidth,
      maxHeight: Math.min(480, Dimensions.get('window').height - SCREEN_MARGIN * 2),
      ...(direction === 'down' ? { top: TOUCH_HEIGHT_POINTER + 6 } : { bottom: TOUCH_HEIGHT_POINTER + 6 }),
    }, 'anchored')}
    </View>
  );
}

const styles = StyleSheet.create({
  panel: {
    borderRadius: 12, borderWidth: 1, zIndex: 1,
    shadowColor: '#000', shadowOpacity: 0.14, shadowRadius: 18, shadowOffset: { width: 0, height: 6 }, elevation: 8,
  },
  sheet: { borderTopLeftRadius: 16, borderTopRightRadius: 16 },
  anchored: { borderRadius: 12 },
  panelHead: {
    flexDirection: 'row', justifyContent: 'space-between', alignItems: 'center',
    paddingHorizontal: 16, paddingVertical: 10, borderBottomWidth: 1,
  },
  panelTitle: { fontSize: 16, fontWeight: '700', flexShrink: 1 },
  panelDone: { minHeight: 44, justifyContent: 'center', paddingHorizontal: 12, flexShrink: 0 },
  panelDoneText: { fontWeight: '600' },
  panelBodyScroll: { flexShrink: 1 },
  panelBody: { padding: 16, gap: 12 },
});
