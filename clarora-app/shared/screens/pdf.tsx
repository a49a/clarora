import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { FlatList, Image, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useAppTheme } from '../ui/ThemeContext';
import { DocumentPicker } from '../services/platform';
import {
  importPdf,
  listPdfLibrary,
  openPdf,
  renderPdfPage,
  type LibraryEntry,
  type PdfMeta,
  type PdfOutlineNode,
} from '../services/pdf';

const BASE_WIDTH = 720;

type FlatOutlineRow = { node: PdfOutlineNode; depth: number; key: string };

function flattenOutline(nodes: PdfOutlineNode[], depth = 0, prefix = '', out: FlatOutlineRow[] = []) {
  nodes.forEach((node, index) => {
    const key = `${prefix}${index}`;
    out.push({ node, depth, key });
    if (node.children?.length) flattenOutline(node.children, depth + 1, `${key}-`, out);
  });
  return out;
}

export default function PdfScreen() {
  const { theme } = useAppTheme();
  const styles = StyleSheet.create({
    page: { flex: 1, backgroundColor: theme.bg },
    content: { maxWidth: 920, width: '100%', alignSelf: 'center', padding: 24, gap: 16 },
    title: { fontSize: 30, fontWeight: '700', color: theme.text },
    hint: { color: theme.textSecondary, lineHeight: 22 },
    row: { flexDirection: 'row', flexWrap: 'wrap', gap: 10, alignItems: 'center' },
    button: { paddingHorizontal: 16, paddingVertical: 12, borderRadius: 10, backgroundColor: theme.surfaceHover },
    labelText: { color: theme.text, fontSize: 15, fontWeight: '600' },
    status: { color: theme.accent },
    entry: { borderWidth: 1, borderColor: theme.border, borderRadius: 10, padding: 14, backgroundColor: theme.surface },
    entryName: { color: theme.text, fontSize: 15, fontWeight: '600' },
  });

  const [library, setLibrary] = useState<LibraryEntry[]>([]);
  const [reading, setReading] = useState<LibraryEntry | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('');

  const refresh = useCallback(async () => setLibrary(await listPdfLibrary()), []);
  useEffect(() => { refresh().catch(() => {}); }, [refresh]);

  const importFile = async () => {
    const picked = await DocumentPicker.getDocumentAsync({ type: 'application/pdf', copyToCacheDirectory: true });
    if (picked.canceled || !picked.assets?.length) return;
    const asset = picked.assets[0];
    setBusy(true);
    try {
      const entry = await importPdf(asset.uri, asset.name);
      await refresh();
      setStatus(`已导入 ${asset.name}`);
      setReading(entry);
    } catch (error) {
      setStatus((error as Error).message || '导入失败');
    } finally {
      setBusy(false);
    }
  };

  if (reading) {
    return (
      <PdfReader
        path={reading.path}
        name={reading.name}
        onBack={() => { setReading(null); void refresh(); }}
      />
    );
  }

  return (
    <View style={styles.page}>
      <ScrollView contentContainerStyle={styles.content}>
        <Text style={styles.title}>文档阅读</Text>
        <Text style={styles.hint}>导入 PDF 学习资料，带目录侧栏的整页阅读器。渲染在本机完成，文件只保存在你的设备。</Text>
        <View style={styles.row}>
          <Pressable accessibilityRole="button" disabled={busy} onPress={() => void importFile()}
            style={[styles.button, busy && { opacity: 0.5 }]}>
            <Text style={styles.labelText}>导入 PDF</Text>
          </Pressable>
          {status ? <Text style={styles.status}>{status}</Text> : null}
        </View>
        {library.length === 0
          ? <Text style={styles.hint}>还没有导入任何 PDF。点「导入 PDF」选择一份学习资料开始阅读。</Text>
          : library.map(entry => (
            <Pressable key={entry.path} accessibilityRole="button" disabled={busy}
              onPress={() => setReading(entry)} style={styles.entry}>
              <Text style={styles.entryName}>{entry.name}</Text>
            </Pressable>
          ))}
      </ScrollView>
    </View>
  );
}

function PdfReader({ path, name, onBack }: { path: string; name: string; onBack: () => void }) {
  const { theme } = useAppTheme();
  const styles = StyleSheet.create({
    page: { flex: 1, backgroundColor: theme.bg },
    bar: { flexDirection: 'row', alignItems: 'center', gap: 12, padding: 12, borderBottomWidth: 1, borderColor: theme.border },
    barText: { color: theme.text, fontSize: 14, fontWeight: '600', flexShrink: 1 },
    barButton: { paddingHorizontal: 12, paddingVertical: 8, borderRadius: 8, backgroundColor: theme.surfaceHover },
    barButtonText: { color: theme.text, fontSize: 13 },
    pageIndicator: { color: theme.textSecondary, fontSize: 13, fontVariant: ['tabular-nums'] },
    hint: { color: theme.textSecondary, padding: 16 },
    body: { flex: 1, flexDirection: 'row' },
    outline: { width: 260, borderRightWidth: 1, borderColor: theme.border, backgroundColor: theme.surface },
    outlineTitle: { padding: 12, fontSize: 13, fontWeight: '700', color: theme.text },
    outlineRow: { paddingVertical: 8, paddingRight: 10 },
    outlineText: { fontSize: 13, color: theme.text },
    outlinePage: { color: theme.textMuted, fontSize: 11, fontVariant: ['tabular-nums'] },
    outlineEmpty: { padding: 12, fontSize: 12, color: theme.textSecondary },
    content: { flex: 1, alignItems: 'center' },
    pageRow: { marginBottom: 12 },
    placeholder: { flex: 1, alignItems: 'center', justifyContent: 'center', backgroundColor: theme.surface },
    placeholderText: { color: theme.textMuted, fontSize: 22, fontVariant: ['tabular-nums'] },
    error: { padding: 16, color: theme.accent },
  });

  const [meta, setMeta] = useState<PdfMeta | null>(null);
  const [error, setError] = useState('');
  const [outlineOpen, setOutlineOpen] = useState(true);
  const [pageIndex, setPageIndex] = useState(0);
  const [zoom, setZoom] = useState(0);
  const width = Math.round(BASE_WIDTH * Math.pow(1.25, zoom));
  const listRef = useRef<FlatList>(null);

  useEffect(() => {
    setMeta(null);
    setError('');
    setPageIndex(0);
    openPdf(path).then(meta => setMeta(meta)).catch(e => setError(e.message || '打开 PDF 失败'));
  }, [path]);

  const rows = meta?.pages.map(page => ({ width: width, height: Math.round((page.height * width) / page.width) })) ?? [];

  const onViewableItems = useRef(({ viewableItems }: { viewableItems: Array<{ index: number | null }> }) => {
    const first = viewableItems.find(item => item.index != null);
    if (first?.index != null) setPageIndex(first.index);
  }).current;

  const flatOutline = useMemo(
    () => flattenOutline(meta?.outline ?? []),
    [meta?.outline],
  );

  const jumpTo = (page: number) => {
    if (page < 0 || !meta) return;
    listRef.current?.scrollToIndex({ index: Math.min(page, meta.pages.length - 1), animated: true });
  };

  const renderOutlineRow = ({ item }: { item: FlatOutlineRow }) => (
    <Pressable accessibilityRole="button" onPress={() => jumpTo(item.node.page)}
      style={[styles.outlineRow, { paddingLeft: 12 + item.depth * 14 }]}>
      <Text numberOfLines={2} style={styles.outlineText}>
        {item.node.title || '未命名'}{item.node.page >= 0 ? ' ' : ''}
      </Text>
      {item.node.page >= 0 && <Text style={styles.outlinePage}>第 {item.node.page + 1} 页</Text>}
    </Pressable>
  );

  return (
    <View style={styles.page}>
      <View style={styles.bar}>
        <Pressable accessibilityRole="button" onPress={onBack} style={styles.barButton}>
          <Text style={styles.barButtonText}>← 返回</Text>
        </Pressable>
        <Text numberOfLines={1} style={styles.barText}>{name}</Text>
        <Pressable accessibilityRole="button" onPress={() => setOutlineOpen(open => !open)} style={styles.barButton}>
          <Text style={styles.barButtonText}>{outlineOpen ? '隐藏目录' : '目录'}</Text>
        </Pressable>
        <View style={styles.barButton}>
          <Text style={styles.pageIndicator}>{meta ? `${pageIndex + 1} / ${meta.pages.length}` : '…'}</Text>
        </View>
        <Pressable accessibilityRole="button" onPress={() => setZoom(z => Math.max(-2, z - 1))} style={styles.barButton}>
          <Text style={styles.barButtonText}>A−</Text>
        </Pressable>
        <Pressable accessibilityRole="button" onPress={() => setZoom(z => Math.min(3, z + 1))} style={styles.barButton}>
          <Text style={styles.barButtonText}>A+</Text>
        </Pressable>
      </View>
      {error ? <Text style={styles.error}>{error}</Text> : null}
      <View style={styles.body}>
        {outlineOpen && (
          <View style={styles.outline}>
            <Text style={styles.outlineTitle}>目录</Text>
            <FlatList
              data={flatOutline}
              keyExtractor={item => item.key}
              renderItem={renderOutlineRow}
              ListEmptyComponent={<Text style={styles.outlineEmpty}>该 PDF 没有内嵌目录。</Text>}
            />
          </View>
        )}
        <FlatList
          ref={listRef}
          data={rows}
          keyExtractor={(_, index) => String(index)}
          getItemLayout={(_, index) => ({ length: (rows[index]?.height ?? 0) + 12, offset: 0, index })}
          onViewableItemsChanged={onViewableItems}
          viewabilityConfig={{ viewAreaCoveragePercentThreshold: 20 }}
          renderItem={({ index }) => (
            <PdfPage
              path={path}
              index={index}
              width={width}
              height={rows[index]?.height ?? 400}
              active={Math.abs(index - pageIndex) <= 2}
            />
          )}
          ListEmptyComponent={!error && !meta ? <Text style={styles.hint}>正在打开…</Text> : null}
        />
      </View>
    </View>
  );
}

function PdfPage({ path, index, width, height, active }: {
  path: string; index: number; width: number; height: number; active: boolean;
}) {
  const { theme } = useAppTheme();
  const [source, setSource] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let mounted = true;
    setSource(null);
    setFailed(false);
    if (active) {
      renderPdfPage(path, index, width)
        .then(dataUri => { if (mounted) setSource(dataUri); })
        .catch(() => { if (mounted) setFailed(true); });
    }
    return () => { mounted = false; };
  }, [active, path, index, width]);
  const styles = StyleSheet.create({
    row: { width, height, marginBottom: 12, backgroundColor: theme.surface, borderWidth: 1, borderColor: theme.border },
    placeholder: { ...StyleSheet.absoluteFillObject, alignItems: 'center', justifyContent: 'center' },
    placeholderText: { color: theme.textMuted, fontSize: 22, fontVariant: ['tabular-nums'] },
  });
  return (
    <View style={styles.row}>
      {source
        ? <Image source={{ uri: source }} style={{ width, height }} resizeMode="contain" />
        : <View style={styles.placeholder}><Text style={styles.placeholderText}>{failed ? '渲染失败' : index + 1}</Text></View>}
    </View>
  );
}
