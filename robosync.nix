{ lib, rustPlatform, fetchFromGitHub }:

rustPlatform.buildRustPackage rec {
  pname = "robosync";
  version = "1.0.0";

  src = fetchFromGitHub {
    owner = "roethlar";
    repo = "robosync";
    rev = "v${version}";
    sha256 = "REPLACE_WITH_ACTUAL_SHA256";
  };

  cargoSha256 = "REPLACE_WITH_ACTUAL_SHA256";

  meta = with lib; {
    description = "High-performance file synchronization with intelligent concurrent processing";
    homepage = "https://github.com/roethlar/robosync";
    license = licenses.mit;
    maintainers = with maintainers; [ ];
  };
}