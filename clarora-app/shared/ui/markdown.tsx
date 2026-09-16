import type { ReactNode } from "react";
import { Linking, StyleSheet, Text, View } from "react-native";
import type { Theme } from "./theme";

type InlinePart = { text: string; bold?: boolean; italic?: boolean; code?: boolean; link?: string };

// Inline tokens: **bold**, *italic*, `code`, [text](url)
const INLINE_SOURCE = "(\\*\\*[^*]+\\*\\*|\\*[^*\\n]+\\*|`[^`\\n]+`|\\[[^\\]\\n]+\\]\\([^)\\n]+\\))";

function parseInline(text: string): InlinePart[] {
  const parts: InlinePart[] = [];
  const pattern = new RegExp(INLINE_SOURCE, "g");
  let lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    const token = match[0];
    if (match.index > lastIndex) {
      parts.push({ text: text.slice(lastIndex, match.index) });
    }
    if (token.startsWith("**")) {
      parts.push({ text: token.slice(2, -2), bold: true });
    } else if (token.startsWith("*")) {
      parts.push({ text: token.slice(1, -1), italic: true });
    } else if (token.startsWith("`")) {
      parts.push({ text: token.slice(1, -1), code: true });
    } else {
      const link = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(token);
      if (link) parts.push({ text: link[1], link: link[2] });
      else parts.push({ text: token });
    }
    lastIndex = match.index + token.length;
  }
  if (lastIndex < text.length) parts.push({ text: text.slice(lastIndex) });
  return parts;
}

function renderSingleTextBlocks(
  text: string,
  styles: ReturnType<typeof makeStyles>
): ReactNode[] {
  const blocks: ReactNode[] = [];
  let codeBuffer: string[] | null = null;
  let key = 0;
  const append = (node: ReactNode) => {
    if (blocks.length > 0) blocks.push("\n");
    blocks.push(node);
  };
  const renderInline = (raw: string): ReactNode[] =>
    parseInline(raw).map((part, i) => (
      <Text
        key={i}
        style={[
          styles.inline,
          part.bold && styles.bold,
          part.italic && styles.italic,
          part.code && styles.code,
          part.link && styles.link,
        ]}
      >
        {part.text}
      </Text>
    ));

  for (const line of text.replace(/\r\n/g, "\n").split("\n")) {
    const trimmed = line.trim();
    if (codeBuffer !== null) {
      if (trimmed.startsWith("```")) {
        append(
          <Text key={key++} style={styles.codeBlockText}>
            {codeBuffer.join("\n")}
          </Text>
        );
        codeBuffer = null;
      } else {
        codeBuffer.push(line);
      }
      continue;
    }
    if (trimmed.startsWith("```")) {
      codeBuffer = [];
      continue;
    }
    if (!trimmed) {
      blocks.push("\n");
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(trimmed);
    if (heading) {
      const level = heading[1].length;
      append(
        <Text
          key={key++}
          style={[
            styles.heading,
            level <= 1 && styles.heading1,
            level === 2 && styles.heading2,
            level >= 3 && styles.heading3,
          ]}
        >
          {renderInline(heading[2])}
        </Text>
      );
      continue;
    }
    if (/^(-{3,}|\*{3,})$/.test(trimmed)) {
      append(<Text key={key++} style={styles.rule}>────────────</Text>);
      continue;
    }
    const listMatch = /^([-*+]|\d+[.)])\s+(.*)$/.exec(trimmed);
    if (listMatch) {
      const bullet = /^[-*+]$/.test(listMatch[1])
        ? "•"
        : `${listMatch[1].replace(/[.)]$/, "")}.`;
      append(
        <Text key={key++} style={styles.inline}>
          <Text style={[styles.inline, styles.bullet]}>{bullet} </Text>
          {renderInline(listMatch[2])}
        </Text>
      );
      continue;
    }
    const quoteMatch = /^>\s?(.*)$/.exec(trimmed);
    if (quoteMatch) {
      append(
        <Text key={key++} style={[styles.inline, styles.quoteText]}>
          │ {renderInline(quoteMatch[1])}
        </Text>
      );
      continue;
    }
    append(
      <Text key={key++} style={[styles.inline, styles.paragraph]}>
        {renderInline(line)}
      </Text>
    );
  }

  if (codeBuffer !== null && codeBuffer.length > 0) {
    append(
      <Text key={key} style={[styles.inline, styles.code, styles.codeBlockText]}>
        {codeBuffer.join("\n")}
      </Text>
    );
  }
  return blocks;
}

export function MarkdownView({
  text,
  theme,
  baseFontSize = 20,
  singleText = false,
}: {
  text: string;
  theme: Theme;
  baseFontSize?: number;
  singleText?: boolean;
}) {
  const styles = makeStyles(theme, baseFontSize);
  if (singleText) {
    return <Text selectable style={styles.singleRoot}>{renderSingleTextBlocks(text, styles)}</Text>;
  }
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const nodes: ReactNode[] = [];
  let codeBuffer: string[] | null = null;
  let key = 0;

  const renderInline = (raw: string, extra?: object): ReactNode[] => {
    return parseInline(raw).map((part, i) => {
      if (part.link) {
        const url = part.link;
        return (
          <Text
            key={i}
            style={[styles.inline, styles.link, extra]}
            onPress={() => Linking.openURL(url)}
          >
            {part.text}
          </Text>
        );
      }
      return (
        <Text
          key={i}
          style={[
            styles.inline,
            part.bold && styles.bold,
            part.italic && styles.italic,
            part.code && styles.code,
            extra,
          ]}
        >
          {part.text}
        </Text>
      );
    });
  };

  for (const line of lines) {
    const trimmed = line.trim();

    if (codeBuffer !== null) {
      if (trimmed.startsWith("```")) {
        nodes.push(
          <View key={key++} style={styles.codeBlock}>
            <Text style={[styles.inline, styles.code, styles.codeBlockText]}>
              {codeBuffer.join("\n")}
            </Text>
          </View>
        );
        codeBuffer = null;
      } else {
        codeBuffer.push(line);
      }
      continue;
    }
    if (trimmed.startsWith("```")) {
      codeBuffer = [];
      continue;
    }
    if (!trimmed) {
      nodes.push(<View key={key++} style={styles.spacer} />);
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(trimmed);
    if (heading) {
      const level = heading[1].length;
      nodes.push(
        <Text
          key={key++}
          style={[
            styles.heading,
            level <= 1 && styles.heading1,
            level === 2 && styles.heading2,
            level >= 3 && styles.heading3,
          ]}
        >
          {renderInline(heading[2])}
        </Text>
      );
      continue;
    }

    if (/^(-{3,}|\*{3,})$/.test(trimmed)) {
      nodes.push(<View key={key++} style={styles.rule} />);
      continue;
    }

    const listMatch = /^([-*+]|\d+[.)])\s+(.*)$/.exec(trimmed);
    if (listMatch) {
      const bullet = /^[-*+]$/.test(listMatch[1]) ? "•" : `${listMatch[1].replace(/[.)]$/, "")}.`;
      nodes.push(
        <View key={key++} style={styles.listItem}>
          <Text style={[styles.inline, styles.bullet]}>{bullet}</Text>
          <Text style={[styles.inline, styles.listText]}>{renderInline(listMatch[2])}</Text>
        </View>
      );
      continue;
    }

    const quoteMatch = /^>\s?(.*)$/.exec(trimmed);
    if (quoteMatch) {
      nodes.push(
        <View key={key++} style={styles.quote}>
          <Text style={[styles.inline, styles.quoteText]}>{renderInline(quoteMatch[1])}</Text>
        </View>
      );
      continue;
    }

    nodes.push(<Text key={key++} style={styles.paragraph}>{renderInline(line)}</Text>);
  }

  if (codeBuffer !== null && codeBuffer.length > 0) {
    nodes.push(
      <View key={`code-${key}`} style={styles.codeBlock}>
        <Text style={[styles.inline, styles.code, styles.codeBlockText]}>
          {codeBuffer.join("\n")}
        </Text>
      </View>
    );
  }

  return <View style={styles.root}>{nodes}</View>;
}

function makeStyles(theme: Theme, base: number) {
  return StyleSheet.create({
    root: { width: "100%" },
    singleRoot: { width: "100%", color: theme.text, fontSize: base, lineHeight: base * 1.55 },
    inline: { color: theme.text, fontSize: base, lineHeight: base * 1.55 },
    paragraph: { marginBottom: base * 0.45 },
    spacer: { height: base * 0.4 },
    bold: { fontWeight: "700" },
    italic: { fontStyle: "italic" },
    code: { fontFamily: "Menlo", fontSize: base * 0.9, backgroundColor: theme.surfaceHover, color: theme.accent },
    link: { color: theme.accent, textDecorationLine: "underline" },
    heading: { fontWeight: "700", color: theme.text },
    heading1: { fontSize: base * 1.35, marginTop: base * 0.6, marginBottom: base * 0.35 },
    heading2: { fontSize: base * 1.18, marginTop: base * 0.5, marginBottom: base * 0.3 },
    heading3: { fontSize: base * 1.05, marginTop: base * 0.4, marginBottom: base * 0.25 },
    rule: { height: 1, backgroundColor: theme.border, marginVertical: base * 0.5 },
    listItem: { flexDirection: "row", marginBottom: base * 0.3, paddingRight: 4 },
    bullet: { marginRight: 8, color: theme.accent, fontWeight: "700" },
    listText: { flex: 1 },
    quote: {
      borderLeftWidth: 3,
      borderLeftColor: theme.accent,
      paddingLeft: 12,
      paddingVertical: 2,
      marginBottom: base * 0.4,
    },
    quoteText: { color: theme.textSecondary, fontStyle: "italic" },
    codeBlock: {
      backgroundColor: theme.surfaceHover,
      borderRadius: 6,
      padding: 10,
      marginBottom: base * 0.45,
    },
    codeBlockText: { color: theme.text },
  });
}
