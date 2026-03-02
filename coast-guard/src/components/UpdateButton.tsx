import { useState, useRef, useEffect, useCallback } from 'react';
import { ArrowsClockwise, Download, CheckCircle } from '@phosphor-icons/react';
import { useTranslation } from 'react-i18next';
import { useUpdateCheck, useApplyUpdateMutation } from '../api/hooks';
import { api } from '../api/endpoints';

type UpdatePhase = 'idle' | 'confirm' | 'downloading' | 'installing' | 'restarting' | 'reconnecting' | 'success' | 'error';

const BASE = 'fixed bottom-4 left-4 z-50';
const PILL = 'inline-flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium shadow-sm transition-colors';

export default function UpdateButton() {
  const { t } = useTranslation();
  const { data: updateInfo } = useUpdateCheck();
  const applyUpdate = useApplyUpdateMutation();
  const [phase, setPhase] = useState<UpdatePhase>('idle');
  const [errorMsg, setErrorMsg] = useState('');
  const popoverRef = useRef<HTMLDivElement>(null);

  // Close popover on outside click
  useEffect(() => {
    if (phase !== 'confirm') return;
    function handleClick(e: MouseEvent) {
      if (popoverRef.current != null && !popoverRef.current.contains(e.target as Node)) {
        setPhase('idle');
      }
    }
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [phase]);

  // Poll for daemon reconnection after update
  const pollForReconnect = useCallback(async (expectedVersion: string) => {
    setPhase('reconnecting');
    const maxAttempts = 30;
    for (let i = 0; i < maxAttempts; i++) {
      await new Promise((r) => setTimeout(r, 2000));
      try {
        const check = await api.checkUpdate();
        if (check.current_version !== expectedVersion) continue;
        setPhase('success');
        setTimeout(() => setPhase('idle'), 5000);
        return;
      } catch {
        // Daemon not ready yet
      }
    }
    setPhase('error');
    setErrorMsg(t('update.terminalFallback'));
  }, [t]);

  const handleUpdate = useCallback(async () => {
    if (updateInfo?.latest_version == null) return;
    setPhase('downloading');

    try {
      const result = await applyUpdate.mutateAsync();
      if (result.success) {
        setPhase('restarting');
        void pollForReconnect(result.version);
      }
    } catch (e) {
      setPhase('error');
      setErrorMsg(e instanceof Error ? e.message : String(e));
    }
  }, [updateInfo, applyUpdate, pollForReconnect]);

  if (updateInfo == null) return null;

  const currentVersion = updateInfo.current_version;
  const latestVersion = updateInfo.latest_version ?? currentVersion;
  const isUpdating = phase === 'downloading' || phase === 'installing' || phase === 'restarting' || phase === 'reconnecting';

  // Success
  if (phase === 'success') {
    return (
      <div className={BASE}>
        <span className={`${PILL} bg-green-50 text-green-700 dark:bg-green-900/30 dark:text-green-400`}>
          <CheckCircle size={14} weight="bold" />
          {t('update.success', { version: latestVersion })}
        </span>
      </div>
    );
  }

  // Error
  if (phase === 'error') {
    return (
      <div className={BASE}>
        <button
          onClick={() => setPhase('idle')}
          className={`${PILL} bg-red-50 text-red-600 dark:bg-red-900/30 dark:text-red-400 cursor-pointer`}
          title={errorMsg}
        >
          {t('update.error', { error: errorMsg })}
        </button>
      </div>
    );
  }

  // Updating spinner
  if (isUpdating) {
    const label =
      phase === 'downloading' ? t('update.downloading') :
      phase === 'installing' ? t('update.installing') :
      phase === 'restarting' ? t('update.restarting') :
      t('update.reconnecting');

    return (
      <div className={BASE}>
        <span className={`${PILL} bg-[var(--surface-muted)] text-[var(--text-muted)]`}>
          <ArrowsClockwise size={14} className="animate-spin" />
          {label}
        </span>
      </div>
    );
  }

  // No update — quiet version label
  if (!updateInfo.update_available) {
    return (
      <div className={BASE}>
        <span
          className={`${PILL} bg-[var(--surface-muted)] text-[var(--text-muted)]`}
          title={`coast ${currentVersion}`}
        >
          v{currentVersion}
        </span>
      </div>
    );
  }

  // Update available — pill with popover
  return (
    <div ref={popoverRef} className={BASE}>
      <button
        onClick={() => setPhase(phase === 'confirm' ? 'idle' : 'confirm')}
        className={`${PILL} bg-amber-50 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400 hover:bg-amber-100 dark:hover:bg-amber-900/50 cursor-pointer`}
        title={`${currentVersion} → ${latestVersion}`}
      >
        <Download size={14} weight="bold" />
        Update available
      </button>

      {phase === 'confirm' && (
        <div className="absolute left-0 bottom-full mb-2 glass-panel p-3 min-w-[240px] z-50 rounded-lg shadow-lg">
          <p className="text-sm text-main mb-3">
            {t('update.confirmTitle', { version: latestVersion })}
          </p>
          <div className="flex gap-2 justify-end">
            <button
              onClick={() => setPhase('idle')}
              className="px-3 py-1.5 text-xs rounded-md text-[var(--text-muted)] hover:bg-[var(--surface-muted-hover)] transition-colors cursor-pointer"
            >
              {t('action.cancel')}
            </button>
            <button
              onClick={() => void handleUpdate()}
              className="px-3 py-1.5 text-xs rounded-md bg-amber-600 text-white hover:bg-amber-700 transition-colors cursor-pointer"
            >
              {t('update.confirm')}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
