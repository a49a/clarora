import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { getSetting, listListeningPractices, setSetting, type ListeningPractice } from '../data/database';

export type ListeningSelection = {
  initialPracticeId?: string | null;
  initialAudioId?: string | null;
  onSelectionChange?: (practiceId: string | null, audioId: string | null) => void;
};

const LAST_LISTENING_SELECTION_SETTING = 'last_listening_selection';

export function useListeningLibrary(
  { initialPracticeId = null, initialAudioId = null, onSelectionChange }: ListeningSelection,
  setError: (message: string | null) => void,
) {
  const readRevision = useRef(0);
  const [practices, setPractices] = useState<ListeningPractice[]>([]);
  const [libraryLoaded, setLibraryLoaded] = useState(false);
  const [selectedPracticeId, setSelectedPracticeId] = useState<string | null>(initialPracticeId);
  const [selectedAudioId, setSelectedAudioId] = useState<string | null>(initialAudioId);
  const [selectionRestored, setSelectionRestored] = useState(initialPracticeId !== null || initialAudioId !== null);
  const selectedPractice = useMemo(
    () => practices.find((p) => p.id === selectedPracticeId) ?? null,
    [practices, selectedPracticeId],
  );

  const selectedAudio = useMemo(() => {
    if (!selectedPractice) return null;
    return selectedPractice.audios.find((a) => a.id === selectedAudioId) ?? null;
  }, [selectedPractice, selectedAudioId]);

  // Restore the last audio before persisting anything new. Without this gate,
  // the first null render after an app restart would overwrite the saved IDs.
  useEffect(() => {
    let cancelled = false;

    if (initialPracticeId !== null || initialAudioId !== null) {
      setSelectionRestored(true);
      return () => {
        cancelled = true;
      };
    }

    void (async () => {
      try {
        const raw = await getSetting(LAST_LISTENING_SELECTION_SETTING);
        if (!raw || cancelled) return;
        const saved: unknown = JSON.parse(raw);
        if (!saved || typeof saved !== 'object') return;
        const record = saved as { practiceId?: unknown; audioId?: unknown };
        const practiceId = typeof record.practiceId === 'string' ? record.practiceId : null;
        const audioId = practiceId && typeof record.audioId === 'string' ? record.audioId : null;
        if (!cancelled) {
          setSelectedPracticeId(practiceId);
          setSelectedAudioId(audioId);
        }
      } catch {
        // A malformed or unavailable setting should not block the library.
      } finally {
        if (!cancelled) setSelectionRestored(true);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [initialAudioId, initialPracticeId]);

  useEffect(() => {
    if (!selectionRestored) return;
    onSelectionChange?.(selectedPracticeId, selectedAudioId);
    void setSetting(
      LAST_LISTENING_SELECTION_SETTING,
      JSON.stringify({ practiceId: selectedPracticeId, audioId: selectedAudioId }),
    ).catch(() => {});
  }, [onSelectionChange, selectedAudioId, selectedPracticeId, selectionRestored]);

  useEffect(() => {
    if (!libraryLoaded) return;
    if (selectedPracticeId && !selectedPractice) {
      setSelectedPracticeId(null);
      setSelectedAudioId(null);
      return;
    }
    if (selectedAudioId && selectedPractice && !selectedAudio) {
      setSelectedAudioId(null);
    }
  }, [libraryLoaded, selectedAudio, selectedAudioId, selectedPractice, selectedPracticeId]);

  const refreshPractices = useCallback(async () => {
    const revision = ++readRevision.current;
    try {
      const list = await listListeningPractices();
      if (revision !== readRevision.current) return;
      setPractices(list);
      setLibraryLoaded(true);
    } catch (e: any) {
      if (revision === readRevision.current) setError(`加载练习列表失败：${e?.message ?? e}`);
    }
  }, [setError]);

  useEffect(() => {
    void refreshPractices();
    return () => {
      ++readRevision.current;
    };
  }, [refreshPractices]);

  return {
    practices,
    selectedPractice,
    selectedAudio,
    selectedPracticeId,
    selectedAudioId,
    setSelectedPracticeId,
    setSelectedAudioId,
    refreshPractices,
  };
}
