export type OcrPageStatus = {
  pageId: string;
  status: string;
  error: string | null;
  text: string | null;
  sourceName: string | null;
  createdAt: string | null;
};

export type OcrPageSummary = {
  pageId: string;
  status: string;
  sourceName: string | null;
  error: string | null;
  createdAt: string | null;
  textLength: number;
};


export type AsrEngineInfo = {
  backend: string;
  label: string;
  configured: boolean;
  available: boolean;
  reason: string | null;
  model: string;
  device: string;
};

export type AsrEngineList = {
  config: Record<string, string>;
  backends: AsrEngineInfo[];
};


export type SpeakingScore = {
  completeness: number;
  accuracy: number;
  wpm: number;
  missed: string[];
  wrong: string[];
  extra: string[];
};

export type SpeakingAttemptSummary = {
  attemptId: string;
  status: string;
  kind: string;
  reference: string | null;
  transcript: string | null;
  feedback: string | null;
  error: string | null;
  createdAt: string | null;
  score?: SpeakingScore | null;
};

export type SpeakingAttemptDetail = SpeakingAttemptSummary & {
  score: SpeakingScore | null;
};
