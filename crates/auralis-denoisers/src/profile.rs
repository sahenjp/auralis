use serde::Serialize;

/// Product-facing enhancement modes. Each profile selects exactly one engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnhancementProfile {
    LowLatency,
    Balanced,
    MaximumQuality,
}

/// Concrete engine selected by an enhancement profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnhancementEngine {
    Rnnoise,
    UlUnas,
    DeepFilterNet3Ll,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct EnhancementProfileMetadata {
    pub profile: EnhancementProfile,
    pub engine: EnhancementEngine,
    pub acceptance: &'static str,
    pub evidence: &'static str,
}

impl EnhancementProfile {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low-latency" => Some(Self::LowLatency),
            "balanced" => Some(Self::Balanced),
            "maximum-quality" => Some(Self::MaximumQuality),
            _ => None,
        }
    }

    pub fn metadata(self) -> EnhancementProfileMetadata {
        match self {
            Self::LowLatency => EnhancementProfileMetadata {
                profile: self,
                engine: EnhancementEngine::Rnnoise,
                acceptance: "accepted-low-cpu-fallback",
                evidence: "native Windows stability and sub-millisecond inference already measured",
            },
            Self::Balanced => EnhancementProfileMetadata {
                profile: self,
                engine: EnhancementEngine::UlUnas,
                acceptance: "provisional-quality-engine",
                evidence: "best current aggregate frozen-corpus objective result with native Windows stability",
            },
            Self::MaximumQuality => EnhancementProfileMetadata {
                profile: self,
                engine: EnhancementEngine::DeepFilterNet3Ll,
                acceptance: "experimental-realtime-blocked",
                evidence: "48 kHz full-band candidate; blind listening pending and Windows first-inference deadline miss unresolved",
            },
        }
    }

    pub fn cli_denoiser(self) -> &'static str {
        match self.metadata().engine {
            EnhancementEngine::Rnnoise => "rnnoise",
            EnhancementEngine::UlUnas => "ul-unas",
            EnhancementEngine::DeepFilterNet3Ll => "deepfilter",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EnhancementEngine, EnhancementProfile};

    #[test]
    fn profiles_map_to_one_distinct_engine_each() {
        assert_eq!(
            EnhancementProfile::LowLatency.metadata().engine,
            EnhancementEngine::Rnnoise
        );
        assert_eq!(
            EnhancementProfile::Balanced.metadata().engine,
            EnhancementEngine::UlUnas
        );
        assert_eq!(
            EnhancementProfile::MaximumQuality.metadata().engine,
            EnhancementEngine::DeepFilterNet3Ll
        );
    }

    #[test]
    fn parser_accepts_only_public_profile_ids() {
        assert_eq!(
            EnhancementProfile::parse("low-latency"),
            Some(EnhancementProfile::LowLatency)
        );
        assert_eq!(
            EnhancementProfile::parse("balanced"),
            Some(EnhancementProfile::Balanced)
        );
        assert_eq!(
            EnhancementProfile::parse("maximum-quality"),
            Some(EnhancementProfile::MaximumQuality)
        );
        assert_eq!(EnhancementProfile::parse("fast"), None);
    }
}
