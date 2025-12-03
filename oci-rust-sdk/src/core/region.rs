use std::fmt;

/// Oracle Cloud Infrastructure regions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    // Americas
    UsPhoenix1,
    UsAshburn1,
    CaToronto1,
    CaMontreal1,
    SaSaopaulo1,
    SaVinhedo1,
    SaSantiago1,

    // EMEA
    UkLondon1,
    UkCardiff1,
    EuFrankfurt1,
    EuZurich1,
    EuAmsterdam1,
    EuMadrid1,
    EuMilan1,
    EuParis1,
    EuStockholm1,
    MeJeddah1,
    MeDubai1,
    IlJerusalem1,
    AfJohannesburg1,

    // Asia Pacific
    ApTokyo1,
    ApOsaka1,
    ApSeoul1,
    ApChuncheon1,
    ApMumbai1,
    ApHyderabad1,
    ApMelbourne1,
    ApSydney1,
    ApSingapore1,
}

impl Region {
    /// Get the region identifier string
    pub fn id(&self) -> &'static str {
        match self {
            // Americas
            Region::UsPhoenix1 => "us-phoenix-1",
            Region::UsAshburn1 => "us-ashburn-1",
            Region::CaToronto1 => "ca-toronto-1",
            Region::CaMontreal1 => "ca-montreal-1",
            Region::SaSaopaulo1 => "sa-saopaulo-1",
            Region::SaVinhedo1 => "sa-vinhedo-1",
            Region::SaSantiago1 => "sa-santiago-1",

            // EMEA
            Region::UkLondon1 => "uk-london-1",
            Region::UkCardiff1 => "uk-cardiff-1",
            Region::EuFrankfurt1 => "eu-frankfurt-1",
            Region::EuZurich1 => "eu-zurich-1",
            Region::EuAmsterdam1 => "eu-amsterdam-1",
            Region::EuMadrid1 => "eu-madrid-1",
            Region::EuMilan1 => "eu-milan-1",
            Region::EuParis1 => "eu-paris-1",
            Region::EuStockholm1 => "eu-stockholm-1",
            Region::MeJeddah1 => "me-jeddah-1",
            Region::MeDubai1 => "me-dubai-1",
            Region::IlJerusalem1 => "il-jerusalem-1",
            Region::AfJohannesburg1 => "af-johannesburg-1",

            // Asia Pacific
            Region::ApTokyo1 => "ap-tokyo-1",
            Region::ApOsaka1 => "ap-osaka-1",
            Region::ApSeoul1 => "ap-seoul-1",
            Region::ApChuncheon1 => "ap-chuncheon-1",
            Region::ApMumbai1 => "ap-mumbai-1",
            Region::ApHyderabad1 => "ap-hyderabad-1",
            Region::ApMelbourne1 => "ap-melbourne-1",
            Region::ApSydney1 => "ap-sydney-1",
            Region::ApSingapore1 => "ap-singapore-1",
        }
    }

    /// Get the endpoint for a specific service in this region
    pub fn endpoint(&self, service: &str) -> String {
        format!("https://{}.{}.oci.oraclecloud.com", service, self.id())
    }

    /// Get the endpoint for the Resource Search service in this region.
    ///
    /// The Resource Search service uses a different endpoint pattern than other services,
    /// with the `query` subdomain instead of a service-specific subdomain.
    ///
    /// # Example
    ///
    /// ```
    /// use oci_rust_sdk::core::region::Region;
    ///
    /// let endpoint = Region::ApSeoul1.query_endpoint();
    /// assert_eq!(endpoint, "https://query.ap-seoul-1.oci.oraclecloud.com");
    /// ```
    pub fn query_endpoint(&self) -> String {
        format!("https://query.{}.oci.oraclecloud.com", self.id())
    }
}

impl std::str::FromStr for Region {
    type Err = crate::core::OciError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            // Americas
            "us-phoenix-1" => Ok(Region::UsPhoenix1),
            "us-ashburn-1" => Ok(Region::UsAshburn1),
            "ca-toronto-1" => Ok(Region::CaToronto1),
            "ca-montreal-1" => Ok(Region::CaMontreal1),
            "sa-saopaulo-1" => Ok(Region::SaSaopaulo1),
            "sa-vinhedo-1" => Ok(Region::SaVinhedo1),
            "sa-santiago-1" => Ok(Region::SaSantiago1),

            // EMEA
            "uk-london-1" => Ok(Region::UkLondon1),
            "uk-cardiff-1" => Ok(Region::UkCardiff1),
            "eu-frankfurt-1" => Ok(Region::EuFrankfurt1),
            "eu-zurich-1" => Ok(Region::EuZurich1),
            "eu-amsterdam-1" => Ok(Region::EuAmsterdam1),
            "eu-madrid-1" => Ok(Region::EuMadrid1),
            "eu-milan-1" => Ok(Region::EuMilan1),
            "eu-paris-1" => Ok(Region::EuParis1),
            "eu-stockholm-1" => Ok(Region::EuStockholm1),
            "me-jeddah-1" => Ok(Region::MeJeddah1),
            "me-dubai-1" => Ok(Region::MeDubai1),
            "il-jerusalem-1" => Ok(Region::IlJerusalem1),
            "af-johannesburg-1" => Ok(Region::AfJohannesburg1),

            // Asia Pacific
            "ap-tokyo-1" => Ok(Region::ApTokyo1),
            "ap-osaka-1" => Ok(Region::ApOsaka1),
            "ap-seoul-1" => Ok(Region::ApSeoul1),
            "ap-chuncheon-1" => Ok(Region::ApChuncheon1),
            "ap-mumbai-1" => Ok(Region::ApMumbai1),
            "ap-hyderabad-1" => Ok(Region::ApHyderabad1),
            "ap-melbourne-1" => Ok(Region::ApMelbourne1),
            "ap-sydney-1" => Ok(Region::ApSydney1),
            "ap-singapore-1" => Ok(Region::ApSingapore1),

            _ => Err(crate::core::OciError::InvalidRegion(s.to_string())),
        }
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_region_id() {
        assert_eq!(Region::ApSeoul1.id(), "ap-seoul-1");
        assert_eq!(Region::UsPhoenix1.id(), "us-phoenix-1");
    }

    #[test]
    fn test_region_endpoint() {
        assert_eq!(
            Region::ApSeoul1.endpoint("osmh"),
            "https://osmh.ap-seoul-1.oci.oraclecloud.com"
        );
    }

    #[test]
    fn test_region_from_str() {
        assert_eq!(Region::from_str("ap-seoul-1").unwrap(), Region::ApSeoul1);
        assert_eq!(
            Region::from_str("us-phoenix-1").unwrap(),
            Region::UsPhoenix1
        );
        assert!(Region::from_str("invalid-region").is_err());
    }

    #[test]
    fn test_region_display() {
        assert_eq!(format!("{}", Region::ApSeoul1), "ap-seoul-1");
    }
}
