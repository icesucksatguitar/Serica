import React from 'react';

interface SwitchProps {
  checked: boolean;
  label: string;
  onChange: (next: boolean) => void;
  disabled?: boolean;
}

/** On/off switch used across Settings and panels. The knob moves with a transform only. */
export const Switch: React.FC<SwitchProps> = ({ checked, label, onChange, disabled }) => (
  <button
    type="button"
    role="switch"
    aria-checked={checked}
    aria-label={label}
    disabled={disabled}
    className={`settings-switch${checked ? ' on' : ''}`}
    onClick={() => onChange(!checked)}
  >
    <span className="settings-switch-thumb" aria-hidden="true" />
  </button>
);
