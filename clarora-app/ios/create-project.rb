# Run only when recreating the checked-in Xcode project: ruby create-project.rb
require 'xcodeproj'
Dir.chdir(__dir__)
project = Xcodeproj::Project.new('Clarora.xcodeproj')
target = project.new_target(:application, 'Clarora-iOS', :ios, '15.1')
target.product_reference.path = 'Clarora.app'
group = project.main_group.new_group('Clarora', 'Clarora')
%w[main.m AppDelegate.mm ClaroraModules.mm].each do |file|
  target.source_build_phase.add_file_reference(group.new_file(file))
end
%w[AppDelegate.h Info.plist].each { |file| group.new_file(file) }
target.build_configurations.each do |config|
  config.build_settings.merge!({
    'PRODUCT_BUNDLE_IDENTIFIER' => 'com.clarora.app',
    'PRODUCT_NAME' => 'Clarora',
    'INFOPLIST_FILE' => 'Clarora/Info.plist',
    'MARKETING_VERSION' => '0.1.0',
    'CURRENT_PROJECT_VERSION' => '1',
    'TARGETED_DEVICE_FAMILY' => '1,2',
    'CODE_SIGN_STYLE' => 'Automatic',
    'CLANG_ENABLE_MODULES' => 'YES',
    'CLANG_ENABLE_OBJC_ARC' => 'YES',
    'OTHER_LDFLAGS' => ['$(inherited)', '-ObjC'],
    'ENABLE_USER_SCRIPT_SANDBOXING' => 'NO',
    'LD_RUNPATH_SEARCH_PATHS' => ['$(inherited)', '@executable_path/Frameworks']
  })
end
phase = target.new_shell_script_build_phase('Bundle React Native code and images')
phase.shell_script = <<~'SH'
  set -e
  export REACT_NATIVE_PATH="${PROJECT_DIR}/../node_modules/react-native"
  WITH_ENVIRONMENT="$REACT_NATIVE_PATH/scripts/xcode/with-environment.sh"
  REACT_NATIVE_XCODE="$REACT_NATIVE_PATH/scripts/react-native-xcode.sh"
  /bin/sh "$WITH_ENVIRONMENT" "$REACT_NATIVE_XCODE"
SH
phase.always_out_of_date = '1'
project.save
scheme = Xcodeproj::XCScheme.new
scheme.add_build_target(target)
scheme.set_launch_target(target)
scheme.save_as(project.path, 'Clarora-iOS', true)
